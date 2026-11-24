"""What the player remembers between runs (2.0): liked songs, volume, the shuffle
and repeat modes, the equalizer and the song that was playing.

One JSON file, `player.json`, next to the saved results (`store.data_dir()`), or
`$REALHIRES_PLAYER`.  The Rust version reads and writes the same file
(`rust/crates/core/src/playerstate.rs`), so both players share likes and EQ.
"""

from __future__ import annotations

import json
import os
import tempfile
from dataclasses import dataclass, field
from typing import List, Optional

from . import eq
from .playqueue import REPEAT_MODES, SHUFFLE_MODES

PLAYER_ENV = "REALHIRES_PLAYER"
FILE_NAME = "player.json"
STATE_VERSION = 1


def default_path() -> str:
    from .store import data_dir

    return os.environ.get(PLAYER_ENV) or os.path.join(data_dir(), FILE_NAME)


@dataclass
class PlayerState:
    liked: List[str] = field(default_factory=list)  # absolute paths, sorted
    volume: float = 0.8                            # 0..1
    muted: bool = False
    shuffle: str = "off"
    repeat: str = "off"
    eq: dict = field(default_factory=eq.default_state)
    last_path: str = ""
    last_position: float = 0.0                     # seconds

    def is_liked(self, path: str) -> bool:
        return path in self.liked

    def set_liked(self, path: str, liked: bool) -> None:
        s = set(self.liked)
        if liked:
            s.add(path)
        else:
            s.discard(path)
        self.liked = sorted(s)

    def to_json(self) -> dict:
        return {"_version": STATE_VERSION, "liked": sorted(set(self.liked)), "volume": self.volume,
                "muted": self.muted, "shuffle": self.shuffle, "repeat": self.repeat, "eq": eq.normalize(self.eq),
                "last": {"path": self.last_path, "position": self.last_position}}

    @classmethod
    def from_json(cls, data) -> "PlayerState":
        st = cls()
        if not isinstance(data, dict):
            return st
        liked = data.get("liked")
        if isinstance(liked, list):
            st.liked = sorted({p for p in liked if isinstance(p, str) and p})
        try:
            st.volume = min(max(float(data.get("volume", st.volume)), 0.0), 1.0)
        except (TypeError, ValueError):
            pass
        st.muted = bool(data.get("muted", False))
        st.shuffle = data.get("shuffle") if data.get("shuffle") in SHUFFLE_MODES else "off"
        # "stop after this song" is for the song playing now, never for the next start
        st.repeat = data.get("repeat") if data.get("repeat") in REPEAT_MODES and data.get("repeat") != "stop" \
            else "off"
        st.eq = eq.normalize(data.get("eq"))
        last = data.get("last")
        if isinstance(last, dict):
            st.last_path = last.get("path") if isinstance(last.get("path"), str) else ""
            try:
                st.last_position = max(float(last.get("position", 0.0)), 0.0)
            except (TypeError, ValueError):
                st.last_position = 0.0
        return st


def load(path: Optional[str] = None) -> PlayerState:
    try:
        with open(path or default_path(), encoding="utf-8") as fh:
            return PlayerState.from_json(json.load(fh))
    except (OSError, ValueError):
        return PlayerState()


def save(state: PlayerState, path: Optional[str] = None) -> str:
    path = path or default_path()
    folder = os.path.dirname(os.path.abspath(path))
    os.makedirs(folder, exist_ok=True)
    fd, tmp = tempfile.mkstemp(prefix=".player-", suffix=".tmp", dir=folder)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as fh:
            json.dump(state.to_json(), fh, ensure_ascii=False, indent=2, sort_keys=True)
            fh.write("\n")
        os.replace(tmp, path)
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise
    return path
