"""The cover that turns over like a record sleeve (2.5, docs/player-spec.md, "2.5", item 2).

On Now Playing the album's songs are the back of the jacket: they are drawn inside the
cover's own square, over the cover mirrored and blurred (`covers.back_picture()`), and
a swipe, a dot or the arrow keys turn the jacket about its vertical axis.  This module
is the part both desktop apps share: the timing and the geometry of the turn
(`turn_frame()`), what a drag does (`drag_progress()`, `drag_finishes()`), and the
measures of the back (`back_layout()`), so the Tk app and the egui app draw the same
thing.  The Rust version is rust/crates/core/src/jacket.rs, checked against
rust/crates/core/tests/python_jacket.json (rust/tools/gen_jacket_vectors.py).
"""

from __future__ import annotations

import math
import os
from dataclasses import dataclass

FLIP_MS = 360            # a whole turn, eased in and out
FADE_MS = 120            # the back's text fades in when the turn has ended
DARKEN = 0.35            # how much a face is darkened when it is edge-on
PERSPECTIVE = 0.06       # how much shorter the far edge is drawn, of the height, when edge-on
SWIPE_PX = 60            # a drag this long (or a quarter of the cover's width) finishes the turn
SCROLL_PX = 30           # a sideways scroll this far turns the jacket (egui; Tk gets whole steps)
HOVER_TINT = 0.12        # the row under the pointer: white at this strength
SECONDARY = 0.70         # the back's secondary texts: white at this strength
SIDES = ("cover", "songs")
NO_ANIMATION_ENV = "REALHIRES_NO_ANIMATION"  # tests: the turn and the fade take no time


def flip_ms() -> int:
    """How long a whole turn takes: FLIP_MS, or 0 with $REALHIRES_NO_ANIMATION."""
    return 0 if os.environ.get(NO_ANIMATION_ENV, "").strip() not in ("", "0") else FLIP_MS


def fade_ms() -> int:
    return 0 if flip_ms() == 0 else FADE_MS


def ease(t: float) -> float:
    """Ease in-out (cubic): 0 -> 0, 0.5 -> 0.5, 1 -> 1, slow at both ends."""
    t = min(max(t, 0.0), 1.0)
    return 4.0 * t * t * t if t < 0.5 else 1.0 - (2.0 - 2.0 * t) ** 3 / 2.0


@dataclass(frozen=True)
class TurnFrame:
    """One moment of the turn.  `other`: the face being turned to is the one seen (past the
    half-way point); `width`: the face's width as a part of the cover's (1 = facing, 0 =
    edge-on); `far_right`: its right edge is the far one, drawn `far` of the height (the
    near edge keeps the whole height, so the jacket never leaves its square); `dark`: how
    much black lies over it."""

    other: bool
    width: float
    far_right: bool
    far: float
    dark: float


def turn_frame(progress: float, direction: int) -> TurnFrame:
    """The jacket `progress` (0..1) of the way through a turn about its vertical axis.
    `direction` > 0: the pointer went to the right (the left edge comes forward and
    crosses to the right), < 0 the other way."""
    p = min(max(progress, 0.0), 1.0)
    angle = math.pi * p
    s = math.sin(angle)
    other = p > 0.5
    return TurnFrame(other=other, width=abs(math.cos(angle)), far_right=(direction > 0) != other,
                     far=1.0 - PERSPECTIVE * s, dark=DARKEN * s)


def drag_progress(dx: float, width: float) -> float:
    """How far a drag of `dx` pixels has turned the jacket: the cover's width is a half turn
    (from one face to the other)."""
    return min(abs(dx) / width, 1.0) if width > 0 else 0.0


def drag_finishes(dx: float, width: float) -> bool:
    """Whether a drag let go at `dx` pixels turns the jacket over (else it springs back):
    past SWIPE_PX or a quarter of the cover's width, whichever is less."""
    return abs(dx) > min(float(SWIPE_PX), width / 4.0)


def rest_ms(progress: float, finish: bool, whole_ms: int = FLIP_MS) -> int:
    """The time the rest of a turn takes when the pointer lets go at `progress`: the part
    of `whole_ms` that is left to the end (`finish`) or back to the start."""
    p = min(max(progress, 0.0), 1.0)
    return int(round(whole_ms * ((1.0 - p) if finish else p)))


def progress_at(start: float, target: float, elapsed_ms: float, total_ms: float) -> float:
    """Where an eased move from `start` to `target` is after `elapsed_ms` of `total_ms`."""
    if total_ms <= 0 or elapsed_ms >= total_ms:
        return target
    return start + (target - start) * ease(max(elapsed_ms, 0.0) / total_ms)


def other_side(side: str) -> str:
    return "cover" if side == "songs" else "songs"


@dataclass(frozen=True)
class BackLayout:
    """The measures of the back of a jacket `size` pixels a side: the album's title (bold,
    `title_px`) at (`pad`, `title_y`), its artist (`small_px`) at `artist_y` (the middles
    of the lines), then `rows` rows of `row_h` from `list_top`; in a row the number ends at
    `num_right`, the title starts at `title_x` and the time ends at `time_right`; the thin
    scroll bar is `bar_w` wide and ends at `bar_right`."""

    size: int
    pad: int
    title_px: int
    text_px: int
    small_px: int
    title_y: int
    artist_y: int
    list_top: int
    row_h: int
    rows: int
    num_right: int
    title_x: int
    time_right: int
    bar_w: int
    bar_right: int


def back_layout(size: int) -> BackLayout:
    size = max(int(size), 1)
    big, mid = size >= 300, size >= 220
    pad = max(8, (size * 5 + 50) // 100)
    title_px = 16 if big else 14 if mid else 12
    text_px = 13 if big else 12 if mid else 11
    small_px = 12 if big else 11 if mid else 10
    row_h = 30 if big else 26 if mid else 22
    title_y = pad + title_px // 2 + 2
    artist_y = title_y + title_px // 2 + small_px // 2 + 5
    list_top = artist_y + small_px // 2 + 8
    bottom = size - max(6, pad // 2)
    rows = max(1, (bottom - list_top) // row_h)
    num_right = pad + text_px * 3 // 2
    return BackLayout(size=size, pad=pad, title_px=title_px, text_px=text_px, small_px=small_px, title_y=title_y,
                      artist_y=artist_y, list_top=list_top, row_h=row_h, rows=rows, num_right=num_right,
                      title_x=num_right + 8, time_right=size - pad, bar_w=3, bar_right=size - 3)


def clamp_first(first: int, total: int, rows: int) -> int:
    """The first row shown, kept so that the list never scrolls past its end."""
    return min(max(first, 0), max(0, total - rows))


def first_for(current: int, first: int, total: int, rows: int) -> int:
    """The first row shown so that row `current` (the song playing) is in view."""
    if current < 0:
        return clamp_first(first, total, rows)
    if current < first:
        first = current
    elif current >= first + rows:
        first = current - rows + 1
    return clamp_first(first, total, rows)


def scroll_bar(first: int, total: int, rows: int, length: int) -> tuple:
    """The thin bar of a list of `total` rows showing `rows` from `first`, in a track
    `length` pixels long: (offset, length) in pixels, or (0, 0) when everything fits."""
    if total <= rows or length <= 0:
        return (0, 0)
    bar = max(12, length * rows // total)
    bar = min(bar, length)
    off = (length - bar) * clamp_first(first, total, rows) // (total - rows)
    return (off, bar)


def mix(a: tuple, b: tuple, amount: float) -> tuple:
    """Colour `b` over colour `a` at `amount` (0..1), each (r, g, b) 0..255, rounded."""
    k = int(round(min(max(amount, 0.0), 1.0) * 256))
    return tuple((x * (256 - k) + y * k + 128) >> 8 for x, y in zip(a, b))


__all__ = ["FLIP_MS", "FADE_MS", "DARKEN", "PERSPECTIVE", "SWIPE_PX", "SCROLL_PX", "HOVER_TINT", "SECONDARY", "SIDES",
           "NO_ANIMATION_ENV", "flip_ms", "fade_ms", "ease", "TurnFrame", "turn_frame", "drag_progress",
           "drag_finishes", "rest_ms", "progress_at", "other_side", "BackLayout", "back_layout", "clamp_first",
           "first_for", "scroll_bar", "mix"]
