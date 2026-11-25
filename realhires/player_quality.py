"""2.5 (docs/player-spec.md, "2.5" item 5) in the Tk player: the quality tooltip where a
song's quality is shown outside the song tables.

Hovering a song's quality shows its resolution and bit rate (`musiclib.quality_detail()`:
"24-bit/96 kHz · 2,304 kbps", "MP3 · 320 kbps · 44.1 kHz").  The song tables show it on
their Quality cells themselves (`SongList`'s `tip_value`, `player_pages.track_tip()`);
this module gives the same tooltip to an area of a canvas (`AreaTip`): the playback
bar's quality badge (`badge_tip()`) and the Format row of Now Playing's Details
(`details_tip()`).
"""

from __future__ import annotations

import tkinter as tk
from typing import Callable, Optional, Tuple

from . import musiclib
from .i18n import tr, tr_text

Box = Tuple[float, float, float, float]


def quality_tip(track) -> str:
    """The quality tooltip of a song in the language in use ("" without a song)."""
    if track is None or getattr(track, "missing", False):
        return ""
    return tr_text(musiclib.quality_detail(track))


class AreaTip:
    """A tooltip for an area of a canvas: `area()` gives its box in canvas coordinates
    (None when it is not shown), `text()` the tooltip.  It appears when the pointer has
    rested on the area for `DELAY_MS`, and goes when the pointer leaves the area or a
    button is pressed."""

    DELAY_MS = 450

    def __init__(self, canvas: tk.Canvas, theme, area: Callable[[], Optional[Box]], text: Callable[[], str]):
        self.c, self.theme, self.area, self.text = canvas, theme, area, text
        self.inside = False
        self.job: Optional[str] = None
        self.win: Optional[tk.Toplevel] = None
        self.tip_text = ""  # the tooltip shown, "" when none
        canvas.bind("<Motion>", self._motion, add="+")
        canvas.bind("<Leave>", lambda e: self.hide(), add="+")
        canvas.bind("<ButtonPress>", lambda e: self.hide(), add="+")

    def _motion(self, e) -> None:
        box = self.area()
        x, y = self.c.canvasx(e.x), self.c.canvasy(e.y)
        inside = box is not None and box[0] <= x <= box[2] and box[1] <= y <= box[3]
        if inside and not self.inside:
            self.inside = True
            rx, ry = e.x_root + 12, e.y_root + 18
            self.job = self.c.after(self.DELAY_MS, lambda: self.show(rx, ry))
        elif not inside and self.inside:
            self.hide()

    def show(self, x: Optional[int] = None, y: Optional[int] = None) -> str:
        """Show the tooltip (near the pointer, else under the area); returns its text."""
        self.job = None
        self._close()
        self.tip_text = text = self.text()
        if not text:
            return ""
        if x is None or y is None:
            box = self.area() or (0, 0, 0, 0)
            x = self.c.winfo_rootx() + int(box[0] - self.c.canvasx(0))
            y = self.c.winfo_rooty() + int(box[3] - self.c.canvasy(0)) + 4
        t = self.theme
        self.win = w = tk.Toplevel(self.c)
        w.wm_overrideredirect(True)
        tk.Label(w, text=text, bg=t.c("lcd"), fg=t.c("text"), font=t.f("small"), padx=8, pady=4,
                 highlightthickness=1, highlightbackground=t.c("sep")).pack()
        w.wm_geometry(f"+{x}+{y}")
        return text

    def _close(self) -> None:
        if self.win is not None:
            try:
                self.win.destroy()
            except tk.TclError:
                pass
            self.win = None

    def hide(self) -> None:
        if self.job is not None:
            try:
                self.c.after_cancel(self.job)
            except tk.TclError:
                pass
            self.job = None
        self._close()
        self.inside = False
        self.tip_text = ""


def badge_tip(lcd) -> AreaTip:
    """The playback bar's quality badge (`Lcd.geo["badge"]`): the song's quality tooltip."""
    return AreaTip(lcd, lcd.app.theme, lambda: lcd.geo.get("badge") if lcd.track is not None else None,
                   lambda: quality_tip(lcd.track))


def details_row(view, label: str) -> Optional[Box]:
    """The box of the row of a DetailsView whose label is `label` (English)."""
    shown = tr(label)
    for item in view.find_withtag("block"):
        if view.type(item) == "text" and view.itemcget(item, "text") == shown:
            box = view.bbox(item)
            x0, _top, x1, _bottom = view.block
            if box and abs(box[0] - x0) < 2:  # a label (the left column), not a value of the same text
                nxt = [b for b in (view.bbox(i) for i in view.find_withtag("block")) if b and b[1] > box[1] + 2
                       and abs(b[0] - x0) < 2]
                bottom = min((b[1] for b in nxt), default=box[3] + 9) - 2
                return (x0 - 4, box[1] - 3, x1 + 4, max(bottom, box[3] + 2))
    return None


def details_tip(view, track: Callable[[], object]) -> AreaTip:
    """Now Playing's Details: the Format row shows the quality tooltip of `track()`."""
    return AreaTip(view, view.theme, lambda: details_row(view, "Format"), lambda: quality_tip(track()))


__all__ = ["AreaTip", "quality_tip", "badge_tip", "details_tip", "details_row"]
