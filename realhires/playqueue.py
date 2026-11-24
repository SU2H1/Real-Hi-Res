"""The play queue of the player (2.0): what plays next, with the shuffle and repeat modes.

Shuffle modes: "off" (the list in its order), "songs" (the songs of the list the
playback was started from, shuffled), "library" (every song in the library),
"genre", "artist" and "album" (the songs that share the current song's genre,
artist or album).  Repeat modes: "off" (stop at the end), "song" (play the
current song again when it ends), "album" (start the current album again when
it ends) and "stop" (stop when the current song ends; it then switches back to
"off").

The shuffled order comes from SplitMix64 and a Fisher-Yates shuffle with a
seed, so the Rust version (`rust/crates/core/src/playqueue.rs`) plays exactly
the same order for the same seed; `rust/crates/core/tests/python_queue.json`
holds this module's answers.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Dict, Iterable, List, Optional

SHUFFLE_MODES = ["off", "songs", "library", "genre", "artist", "album"]
SHUFFLE_LABELS = {"off": "Off", "songs": "Songs in this list", "library": "All songs in the library",
                  "genre": "Same genre", "artist": "Same artist", "album": "Same album"}
REPEAT_MODES = ["off", "song", "album", "stop"]
REPEAT_LABELS = {"off": "Off", "song": "Repeat song", "album": "Repeat album",
                 "stop": "Stop after this song"}
MASK = (1 << 64) - 1


class SplitMix64:
    """A tiny, well-known PRNG whose output both versions reproduce bit for bit."""

    def __init__(self, seed: int):
        self.state = seed & MASK

    def next(self) -> int:
        self.state = (self.state + 0x9E3779B97F4A7C15) & MASK
        z = self.state
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & MASK
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & MASK
        return z ^ (z >> 31)

    def below(self, n: int) -> int:
        """A number in 0..n-1 (n >= 1)."""
        return self.next() % n


def shuffled(items: List[str], rng: SplitMix64) -> List[str]:
    out = list(items)
    for i in range(len(out) - 1, 0, -1):
        j = rng.below(i + 1)
        out[i], out[j] = out[j], out[i]
    return out


@dataclass
class Song:
    """What the queue needs to know about a song."""
    path: str
    title: str = ""
    artist: str = ""
    album: str = ""
    album_artist: str = ""
    genre: str = ""
    disc: int = 0
    track: int = 0

    @property
    def artist_key(self) -> str:
        return (self.album_artist or self.artist).strip().casefold()

    @property
    def album_key(self) -> str:
        return f"{self.album.strip().casefold()}\x00{self.artist_key}"

    @property
    def genre_key(self) -> str:
        return self.genre.strip().casefold()


def album_order(songs: Iterable[Song]) -> List[Song]:
    """Disc, then track number, then title and path (songs without numbers last)."""
    return sorted(songs, key=lambda s: (s.disc or 9999, s.track or 9999, s.title.casefold(), s.path))


def number(value) -> int:
    """'3/12' -> 3; anything unreadable -> 0."""
    text = str(value or "").strip().split("/")[0].strip()
    return int(text) if text.isdigit() else 0


@dataclass
class PlayQueue:
    library: List[Song]
    context: List[str]                       # the list the playback was started from, in its order
    shuffle: str = "off"
    repeat: str = "off"
    seed: int = 0
    order: List[str] = field(default_factory=list)
    pos: int = 0

    def __post_init__(self):
        self.by_path: Dict[str, Song] = {s.path: s for s in self.library}
        self.rng = SplitMix64(self.seed)
        if self.shuffle not in SHUFFLE_MODES:
            self.shuffle = "off"
        if self.repeat not in REPEAT_MODES:
            self.repeat = "off"

    # ---------------------------------------------------------------- start ---
    def start(self, path: str) -> str:
        """Play `path` now and build the order of the songs after it."""
        self.order = [path] + self._upcoming(path)
        self.pos = 0
        return path

    @property
    def current(self) -> Optional[str]:
        return self.order[self.pos] if 0 <= self.pos < len(self.order) else None

    def _pool(self, path: str) -> List[str]:
        """The songs the current shuffle (or repeat album) mode draws from, in list order."""
        cur = self.by_path.get(path)
        mode = self.shuffle
        if self.repeat == "album" and mode not in ("off", "songs", "album"):
            mode = "album"  # repeating the album keeps the playback inside it
        if mode in ("off", "songs"):
            if self.repeat == "album" and cur is not None:
                return [s.path for s in album_order(x for x in self.library if x.album_key == cur.album_key)]
            return list(self.context)
        if mode == "library" or cur is None:
            return [s.path for s in self.library]
        key = {"genre": "genre_key", "artist": "artist_key", "album": "album_key"}[mode]
        want = getattr(cur, key)
        songs = [s for s in self.library if getattr(s, key) == want]
        return [s.path for s in (album_order(songs) if mode == "album" else songs)]

    def _upcoming(self, path: str) -> List[str]:
        pool = self._pool(path)
        if self.shuffle == "off":
            if path in pool:
                i = pool.index(path)
                return pool[i + 1:]
            return pool
        return shuffled([p for p in pool if p != path], self.rng)

    def set_shuffle(self, mode: str) -> None:
        self.shuffle = mode if mode in SHUFFLE_MODES else "off"
        self._rebuild()

    def set_repeat(self, mode: str) -> None:
        self.repeat = mode if mode in REPEAT_MODES else "off"
        if self.repeat == "album":
            self._rebuild()

    def _rebuild(self) -> None:
        cur = self.current
        if cur is not None:
            self.order = self.order[:self.pos + 1] + self._upcoming(cur)

    # ----------------------------------------------------------------- moves ---
    def next(self, auto: bool = False) -> Optional[str]:
        """The song to play next: `auto` when the current one ended by itself
        (repeat song and stop after this song apply), otherwise the Next button.
        None means stop."""
        cur = self.current
        if cur is None:
            return None
        if auto and self.repeat == "song":
            return cur
        if auto and self.repeat == "stop":
            self.repeat = "off"
            return None
        if self.pos + 1 < len(self.order):
            nxt = self.order[self.pos + 1]
            if self.repeat == "album":
                a, b = self.by_path.get(cur), self.by_path.get(nxt)
                if a is not None and b is not None and a.album_key != b.album_key:
                    return self._restart_album(cur)
            self.pos += 1
            return nxt
        if self.repeat == "album":
            return self._restart_album(cur)
        return None

    def _restart_album(self, cur: str) -> Optional[str]:
        song = self.by_path.get(cur)
        if song is None:
            return None
        tracks = [s.path for s in album_order(x for x in self.library if x.album_key == song.album_key)]
        if self.shuffle != "off":
            tracks = shuffled(tracks, self.rng)
        self.order = self.order[:self.pos + 1] + tracks
        self.pos += 1
        return self.order[self.pos]

    def previous(self) -> Optional[str]:
        """The song before the current one (None at the start: play it again)."""
        if self.pos > 0:
            self.pos -= 1
            return self.order[self.pos]
        return None

    def up_next(self, count: int = 50) -> List[str]:
        return self.order[self.pos + 1:self.pos + 1 + count]
