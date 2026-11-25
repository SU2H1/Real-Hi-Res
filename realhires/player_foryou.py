"""For You in the Tk player (2.5, docs/player-spec.md "1. For You: playlists made from
likes, retention and listening time"): the player's side of `recommend`
(`ForYouMixin`, which `PlayerApp` inherits), the For You page (`ForYouPage`: a card per
mix with the mosaic of its first songs, its name, number of songs and description) and
a mix's page (`MixPage`: Play, Shuffle, Save as Playlist and the songs in the mix's
order).

The mixes are worked out from the library, the whole play history and the likes when a
page asks for them, and kept until one of those changes: `foryou_changed()` is called
after a play is logged and after the history is cleared; a like, a dislike, another
library or another day are seen by the cache's key (`foryou_mixes()`)."""

from __future__ import annotations

import time
import tkinter as tk
from typing import TYPE_CHECKING, Callable, Dict, List, Optional, Tuple

from . import recommend
from .i18n import N_, tr
from .musiclib import Track
from .player_collections import ScrollCanvas
from .player_pages import Page, frame, label, plural, song_table, track_value
from .player_theme import IconButton, PillButton, fit_text, round_rect
from .player_widgets import Column, CoverCache, draw_mosaic

if TYPE_CHECKING:  # pragma: no cover
    from .player_gui import PlayerApp

MIX_NAMES = {mid: name for mid, name, _ in recommend.MIXES}
MIX_NOTES = {mid: note for mid, _, note in recommend.MIXES}


class ForYouMixin:
    """The mixes of `recommend`, worked out once per library, history, likes and day."""

    _foryou_ver = 0                        # changes with the play history
    _foryou_cache: Optional[tuple] = None  # (key, the tracks they were made from, the mixes)
    foryou_message = ""                    # what Save as Playlist made last

    def foryou_changed(self: "PlayerApp") -> None:
        """The play history changed: the mixes are made again when a page next shows them."""
        self._foryou_ver += 1
        self._refresh_pages({"foryou", "foryou_mix"})

    def foryou_mixes(self: "PlayerApp") -> List[dict]:
        """The mixes now (`recommend.mixes()`), each {"id", "name", "description", "paths"}."""
        now = time.time()
        state = self.state
        key = (self._foryou_ver, tuple(state.liked), tuple(state.disliked), recommend.day_of(now))
        cache = self._foryou_cache
        if cache is not None and cache[0] == key and cache[1] is self.tracks:
            return cache[2]
        try:
            mixes = recommend.mixes(self.tracks, self.plays(), state.liked, state.disliked, now, key[3])
        except Exception:  # noqa: BLE001 - a broken history must not stop the app
            mixes = []
        self._foryou_cache = (key, self.tracks, mixes)
        return mixes

    def foryou_mix(self: "PlayerApp", mid: str) -> Optional[dict]:
        return next((m for m in self.foryou_mixes() if m["id"] == mid), None)

    def mix_tracks(self: "PlayerApp", mid: str) -> List[Track]:
        m = self.foryou_mix(mid)
        return [self.by_path[p] for p in m["paths"] if p in self.by_path] if m else []

    def foryou_entry_count(self: "PlayerApp") -> str:
        """What the Library page's For You entry says under its name."""
        n = len(self.foryou_mixes())
        if not n:
            return tr("No mixes yet")
        return tr("1 mix") if n == 1 else tr("{n} mixes", n=n)

    def open_mix(self: "PlayerApp", mid: str) -> None:
        self.show_page("foryou_mix", mix=mid)

    def save_mix_playlist(self: "PlayerApp", mid: str) -> Optional[str]:
        """Save as Playlist: an ordinary playlist with the mix's name and songs in its
        order (a taken name gets " 2"); says what was made.  Returns its id (None
        without the mix)."""
        m = self.foryou_mix(mid)
        if m is None or not m["paths"]:
            return None
        pid = self.new_playlist(songs=list(m["paths"]), name=tr(m["name"]), open_page=False)
        p = self.state.playlist(pid) if pid else None
        if p is None:
            return None
        n = len(p["songs"])
        self.foryou_message = (tr("Made the playlist “{name}” with 1 song.", name=p["name"]) if n == 1 else
                               tr("Made the playlist “{name}” with {n} songs.", name=p["name"], n=f"{n:,}"))
        msg = self.foryou_message
        self.sidebar.set_status(msg)
        self.root.after(4000, lambda: self.sidebar.status.cget("text") == msg and self.sidebar.set_status(""))
        page = self.pages.get("foryou_mix")
        if isinstance(page, MixPage):
            page.made.configure(text=msg)
        return pid


# ------------------------------------------------------------------- the pages ---
class ForYouPage(Page):
    """A card per mix: the mosaic of its first songs' covers, its name, "N songs" and
    what it is, in secondary text; a click opens the mix.  Under the title, one line on
    how the mixes are made; without mixes, how to get them."""

    list_name = N_("For You")
    MARGIN, GAP = 28, 24
    COVER = CoverCache.GRID
    TEXT_H = 92  # the name, the number of songs and up to three lines of the description

    def __init__(self, app):
        super().__init__(app)
        self.area = ScrollCanvas(self, self, self._draw)
        self.area.pack(fill="both", expand=True)
        self.c = self.area.c
        self.hits: List[Tuple[tuple, Callable[[], None]]] = []
        self.card_hits: Dict[str, tuple] = {}   # where each card was drawn, by the mix's id (for the tests)
        self.hover: Optional[tuple] = None
        self.mixes: List[dict] = []
        self._fit: dict = {}
        self.c.bind("<Button-1>", self._click)
        self.c.bind("<Motion>", self._motion)
        self.c.bind("<Leave>", lambda e: self._set_hover(None))
        app.covers.listeners.append(lambda key, size: size in (self.COVER, self.COVER // 2 - 1)
                                    and self.winfo_exists() and self.winfo_ismapped() and self.area.redraw())

    def refresh(self) -> None:
        self.mixes = self.app.foryou_mixes()
        self.area.redraw()

    def show(self, **kw) -> None:
        self.refresh()

    def likes_changed(self) -> None:
        self.app._refresh_pages({"foryou"})

    def _draw(self) -> float:
        t, c, app = self.theme, self.c, self.app
        W = c.winfo_width()
        m, cs = self.MARGIN, self.COVER
        self.hits, self.card_hits = [], {}
        c.create_text(m, 40, text=tr("For You"), anchor="w", fill=t.c("text"), font=t.f("title"))
        note = c.create_text(m, 64, text=tr(recommend.HOW_NOTE), anchor="nw", fill=t.c("text2"), font=t.f("body"),
                             width=max(100, W - 2 * m))
        y = c.bbox(note)[3] + 22
        if not self.mixes:
            c.create_text(m, y, text=tr(recommend.EMPTY_NOTE), anchor="nw", fill=t.c("text"), font=t.f("h2"),
                          width=max(100, W - 2 * m))
            return y + 80
        cols = max(1, int((W - 2 * m + self.GAP) // (cs + self.GAP)))
        gap = min(self.GAP * 2.5, (W - 2 * m - cols * cs) / (cols - 1)) if cols > 1 else 0.0
        gap = max(float(self.GAP), gap)
        for i, mix in enumerate(self.mixes):
            r, k = divmod(i, cols)
            x0 = m + k * (cs + gap)
            y0 = y + r * (cs + self.TEXT_H + self.GAP)
            box = (x0, y0, x0 + cs, y0 + cs + self.TEXT_H)
            draw_mosaic(c, t, app.covers, app.playlist_mosaic({"songs": mix["paths"]}), x0, y0, cs)
            if self.hover == box:
                round_rect(c, x0, y0, x0 + cs, y0 + cs, 8, fill="", outline=t.c("accent"), width=2)
            c.create_text(x0, y0 + cs + 16, text=fit_text(t.f("card_title"), tr(mix["name"]), cs, self._fit),
                          anchor="w", fill=t.c("text"), font=t.f("card_title"))
            c.create_text(x0, y0 + cs + 34, text=plural(len(mix["paths"]), "song"), anchor="w", fill=t.c("text2"),
                          font=t.f("card_sub"))
            c.create_text(x0, y0 + cs + 45, text=tr(mix["description"]), anchor="nw", fill=t.c("text2"),
                          font=t.f("small"), width=cs)
            self.hits.append((box, lambda mid=mix["id"]: app.open_mix(mid)))
            self.card_hits[mix["id"]] = box
        rows = (len(self.mixes) + cols - 1) // cols
        return y + rows * (cs + self.TEXT_H + self.GAP) + 10

    def _hit(self, e) -> Optional[tuple]:
        x, y = self.c.canvasx(e.x), self.c.canvasy(e.y)
        return next(((b, fn) for b, fn in self.hits if b[0] <= x <= b[2] and b[1] <= y <= b[3]), None)

    def _click(self, e) -> None:
        hit = self._hit(e)
        if hit:
            hit[1]()

    def _set_hover(self, box: Optional[tuple]) -> None:
        self.c.configure(cursor="hand2" if box else "")
        if box != self.hover:
            self.hover = box
            self.area.redraw()

    def _motion(self, e) -> None:
        hit = self._hit(e)
        self._set_hover(hit[0] if hit else None)

    def songs(self) -> List[Track]:
        """The Play button of the bar with nothing playing: the first mix."""
        return self.app.mix_tracks(self.mixes[0]["id"]) if self.mixes else []


class MixPage(Page):
    """One mix: the mosaic, its name and description, "N songs, M minutes", Play,
    Shuffle and Save as Playlist (an ordinary playlist with the mix's name; the page says
    what it made), then the songs in the mix's order with the usual song menu."""

    COVER = 220

    def __init__(self, app):
        super().__init__(app)
        t = self.theme
        self.mid = ""
        top = frame(self, t)
        top.pack(fill="x", padx=28, pady=(18, 6))
        IconButton(top, t, "back", command=app.go_back, size=30, icon_size=16,
                   tooltip=lambda: tr("Back")).pack(side="left", anchor="n", padx=(0, 6))
        self.cover = tk.Canvas(top, width=self.COVER, height=self.COVER, highlightthickness=0, bd=0)
        t.reg(self.cover, bg="bg")
        self.cover.pack(side="left")
        info = frame(top, t)
        info.pack(side="left", fill="both", expand=True, padx=(24, 0))
        label(info, t, tr("For You"), "small_b", fg="text2").pack(anchor="w", pady=(34, 0))
        self.title_l = label(info, t, "", "big", wraplength=600, justify="left")
        self.title_l.pack(anchor="w", pady=(0, 2))
        self.note_l = label(info, t, "", "body", fg="text2", wraplength=500, justify="left")
        self.note_l.pack(anchor="w")
        self.meta_l = label(info, t, "", "body", fg="text2")
        self.meta_l.pack(anchor="w", pady=(2, 14))
        btns = frame(info, t)
        btns.pack(anchor="w")
        self.play_b = PillButton(btns, t, tr("Play"), icon="play", command=lambda: self._play(False))
        self.play_b.pack(side="left")
        self.shuffle_b = PillButton(btns, t, tr("Shuffle"), icon="shuffle", command=lambda: self._play(True))
        self.shuffle_b.pack(side="left", padx=(10, 0))
        self.save_b = PillButton(btns, t, tr("Save as Playlist"), icon="plus", kind="plain", command=self.save)
        self.save_b.pack(side="left", padx=(18, 0))
        self.made = label(info, t, "", "small", fg="accent", wraplength=500, justify="left")
        self.made.pack(anchor="w", pady=(8, 0))
        info.bind("<Configure>", self._wrap)
        self.pos: Dict[int, int] = {}
        # the place in the mix, then the columns the user chose for song lists (not sortable: the
        # order is the mix's)
        self.table = song_table(self, self, value=self._value, sortable=False, kind="songs",
                                prefix=[Column("pos", "", 34, anchor="e", kind="num")],
                                on_activate=lambda i: self.app.play_list(self.table.rows, i, self.list_name),
                                empty_text=tr("This mix has no songs at the moment. It comes back as you play "
                                              "and like songs."))
        self.table.pack(fill="both", expand=True, padx=(16, 16))
        app.covers.listeners.append(lambda key, size: size in (self.COVER, self.COVER // 2 - 1) and self.winfo_exists()
                                    and self.winfo_ismapped() and self._draw_cover())
        t.on_change(self._draw_cover)

    def _wrap(self, e) -> None:
        width = max(200, e.width - 8)
        for w in (self.title_l, self.note_l, self.made):
            w.configure(wraplength=width)

    def _value(self, row: Track, key: str) -> str:
        if key == "pos":
            return str(self.pos.get(id(row), ""))
        return track_value(row, key)

    def show(self, mix: Optional[str] = None, **kw) -> None:
        if mix is not None and mix != self.mid:
            self.mid = mix
            self.made.configure(text="")
        self.refresh()

    def refresh(self) -> None:
        rows = self.app.mix_tracks(self.mid)
        self.pos = {id(r): i + 1 for i, r in enumerate(rows)}
        self.list_name = tr(MIX_NAMES.get(self.mid, "For You"))
        self.title_l.configure(text=self.list_name)
        self.note_l.configure(text=tr(MIX_NOTES[self.mid]) if self.mid in MIX_NOTES else "")
        minutes = int(round(sum(r.duration for r in rows) / 60))
        self.meta_l.configure(text=tr("{a}, {b}", a=plural(len(rows), "song"), b=plural(minutes, "minute")))
        for b in (self.play_b, self.shuffle_b, self.save_b):
            b.set(enabled=bool(rows))
        self.table.set_rows(rows, keep=True)
        self._draw_cover()

    def _draw_cover(self) -> None:
        self.cover.delete("all")
        paths = [r.path for r in self.table.rows]
        draw_mosaic(self.cover, self.theme, self.app.covers, self.app.playlist_mosaic({"songs": paths}), 0, 0,
                    self.COVER)

    def save(self) -> Optional[str]:
        return self.app.save_mix_playlist(self.mid)

    def songs(self) -> List[Track]:
        return list(self.table.rows)

    def likes_changed(self) -> None:
        # a disliked song leaves the mix, a liked one may enter another: made again
        self.app._refresh_pages({"foryou_mix"})

    def song_changed(self) -> None:
        self.table.redraw()
