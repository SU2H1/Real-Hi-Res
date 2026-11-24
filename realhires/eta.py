"""Estimated time left for a scan.

The estimate counts bytes rather than files: analysing a file takes time in
proportion to its audio, so a library that mixes short and long tracks (or
16/44.1 and 24/192 files) is estimated much better from bytes than from a file
count.  Files whose saved result is reused take no time and are left out, and
parallel workers are covered automatically because the measured throughput is
that of all of them together.
"""

from __future__ import annotations

import os
import time
from typing import Callable, Dict, Iterable, Optional

MIN_FILES = 3       # finished files before an estimate is shown
MIN_SECONDS = 3.0   # seconds of analysis before an estimate is shown


def file_sizes(paths: Iterable[str]) -> Dict[str, int]:
    """{path: size in bytes}; an unreadable file counts as one byte."""
    sizes = {}
    for p in paths:
        try:
            sizes[p] = max(1, os.path.getsize(p))
        except OSError:
            sizes[p] = 1
    return sizes


class ScanEta:
    """Time left for analysing `sizes` (path -> bytes), from the throughput so far."""

    def __init__(self, sizes: Dict[str, int], clock: Callable[[], float] = time.monotonic):
        self.sizes = dict(sizes)
        self.total = sum(self.sizes.values())
        self.clock = clock
        self.start = clock()
        self.done_bytes = 0
        self.done: set = set()

    def add(self, path: str) -> None:
        """One file has been analysed."""
        if path in self.sizes and path not in self.done:
            self.done.add(path)
            self.done_bytes += self.sizes[path]

    def seconds_left(self) -> Optional[float]:
        """None until there is enough to go on."""
        elapsed = self.clock() - self.start
        if (len(self.done) < min(MIN_FILES, len(self.sizes)) or elapsed < MIN_SECONDS
                or self.done_bytes <= 0):
            return None
        return max(0.0, (self.total - self.done_bytes) * elapsed / self.done_bytes)

    def text(self) -> str:
        return fmt_left(self.seconds_left())


def fmt_left(seconds: Optional[float]) -> str:
    """'about 6 min left', 'about 1 h 20 min left', 'less than a minute left'."""
    if seconds is None:
        return "estimating the time left"
    return f"{fmt_about(seconds)} left"


def fmt_about(seconds: float) -> str:
    """'about 6 min', 'about 1 h 20 min', 'less than a minute'."""
    if seconds < 60:
        return "less than a minute"
    minutes = int(seconds // 60 + (1 if seconds % 60 >= 30 else 0))
    if minutes < 60:
        return f"about {minutes} min"
    h, m = divmod(minutes, 60)
    return f"about {h} h {m} min" if m else f"about {h} h"
