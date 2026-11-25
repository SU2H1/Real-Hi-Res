"""2.4 (docs/player-spec.md, "2.4" item 2) in the Tk player: the names on Your Stats as
links, and the leaderboard: a section of Your Stats and the "Most Played" page.

Every song, artist, album and genre named on Your Stats (the cards, the top lists of
Details, the day beside the calendar, the leaderboard) is a link: a song plays it with
the songs of that list as the queue, an artist opens the artist's page, an album its
page, a genre its page.  The pointer is a hand over a link, and a right click on a song
opens the song menu.  The helpers here register links as boxes on a canvas (`hits`:
(box, command), `menus`: (box, command taking the event)); `LinkArea` handles the
clicks, the pointer and the right clicks of a canvas that has no handlers of its own.

The leaderboard ranks the most played songs, artists or albums of the period chosen on
Your Stats (`stats.leaderboard()`): rank (the top three on a gold, silver and bronze
badge), cover, title, artist, plays and minutes listened, up to 100 rows, ties sharing
a rank.  Its switch (Songs, Artists, Albums) is `PlayerApp.leaderboard_kind`, kept for
the session and shared by the section and the page.  Play / Shuffle play the ranked
songs in rank order (`stats.ranked_songs()`).

2.5 (item 4): every card of Your Stats is one click target (`card_target()`: the top
song, artist, album and genre open Most Played with that kind, the busiest day is
chosen in the calendar, the others open Details).  The leaderboard has a fourth kind,
Genres, and a "Sort by" switch (Times played / Time listened: `PlayerApp.leaderboard_by`,
shared by the section and the page; the number sorted by is drawn in the text colour).
Most Played has a period control of its own (`PlayerApp.leaderboard_span`, a unit of
`stats.SPAN_UNITS` and an offset: Day, Week, Month, Year, All time, with ‹ and ›), which
starts from the period of Your Stats when the page is opened from there
(`open_most_played()`).  The switches are drawn on the page's canvas and wrap to a
second line when the page is narrow (`draw_controls()`).
"""

from __future__ import annotations

import time
import tkinter as tk
from tkinter import ttk
from typing import Callable, List, Optional, Sequence, Tuple

from . import musiclib, stats
from .i18n import N_, tr, tr_name, tr_text
from .musiclib import Track
from .player_pages import Page, find_group, known_artists, plural
from .player_theme import draw_icon, fit_text, round_rect
from .player_widgets import draw_cover

MEDALS = ("#d4a017", "#9aa0a6", "#c27a3a")  # the rank badges of the top three: gold, silver, bronze
SECTION_ROWS = 10        # the leaderboard section of Your Stats; the Most Played page shows up to stats.TOP
ROW_H, COVER = 52, 40    # a leaderboard row, its cover
KIND_LABELS = {"songs": N_("Songs"), "artists": N_("Artists"), "albums": N_("Albums"), "genres": N_("Genres")}
# 2.5: "Sort by", and the units of Most Played's period control
SORT_LABELS = {"plays": N_("Times played"), "time": N_("Time listened")}
UNIT_LABELS = {"day": N_("Day"), "week": N_("Week"), "month": N_("Month"), "year": N_("Year"),
               "all": N_("All time")}
SEG_H = 30               # a switch's height
# the cards of Your Stats that open Most Played (with this kind), and Details at a section
CARD_KINDS = {"Your top song": "songs", "Your top artist": "artists", "Your top album": "albums",
              "Your top genre": "genres"}
CARD_SECTIONS = {"Your listening hour": "hour", "Your listening day": "weekday"}

Box = Tuple[float, float, float, float]
Hits = List[Tuple[Box, Callable[[], None]]]
Menus = List[Tuple[Box, Callable[[object], None]]]


# ------------------------------------------------------------------ data ---
def period_plays(app) -> list:
    """The plays of the period chosen on Your Stats."""
    since, until = stats.period_range(app.stats_period, time.time())
    return app.plays(since, until)


def sort_by(app) -> str:
    """The leaderboard's "Sort by" (2.5): "plays" or "time"."""
    by = getattr(app, "leaderboard_by", "plays")
    return by if by in stats.LEADERBOARD_SORTS else "plays"


def span_of(app) -> Tuple[str, int]:
    """The period Most Played shows (2.5): (unit, offset), offset 0 being the present."""
    unit, offset = getattr(app, "leaderboard_span", ("all", 0))
    return (unit, min(0, int(offset))) if unit in stats.SPAN_UNITS and unit != "all" else ("all", 0)


def span_text(unit: str, offset: int, now: Optional[float] = None) -> str:
    """The name of a span in the language in use: "25 November 2026", "23 – 29 November
    2026", "November 2026", "2026", "All time"."""
    title = stats.span_title(unit, offset, time.time() if now is None else now)
    if unit == "year":
        return tr("{year}", year=title)
    return tr(title) if unit == "all" else tr_text(title)


def boards(plays, by: str = "plays") -> dict:
    """The leaderboards of these plays, by kind."""
    plays = list(plays)
    return {kind: stats.leaderboard(plays, kind, by=by) for kind in stats.LEADERBOARD_KINDS}


def first_play(app) -> Optional[float]:
    """When the oldest play of the history started (None without plays)."""
    try:
        return app.lib.first_play()
    except Exception:  # noqa: BLE001 - the history must never stop the page
        return None


def open_most_played(app, kind: Optional[str] = None) -> None:
    """Most Played, opened from Your Stats (See All, a card): with this kind, starting
    from the period chosen there (stats.period_span())."""
    if kind in stats.LEADERBOARD_KINDS:
        app.leaderboard_kind = kind
    app.leaderboard_span = stats.period_span(app.stats_period)
    app.show_page("most_played")


def card_target(page, label: str) -> Callable[[], None]:
    """What a click on a card of Your Stats does (2.5; `label` is the card's English
    label, `page` the StatsPage): the top song, artist, album and genre open Most Played
    with that kind, the biggest day is chosen in the calendar (and scrolled to), the
    others open Details (the listening hour and day at their charts)."""
    app, st = page.app, page.st or {}
    if label in CARD_KINDS:
        return lambda: open_most_played(app, CARD_KINDS[label])
    if label == "Your biggest day" and st.get("busiest_day"):
        try:
            y, m, d = (int(x) for x in st["busiest_day"][0].split("-"))
        except ValueError:
            y = 0
        if y:
            return lambda: page.show_day(y, m, d)
    return lambda: app.show_page("stats_details", section=CARD_SECTIONS.get(label, "top"))


def card_menu(page, label: str) -> Optional[Callable[[object], None]]:
    """A right click on the top song's card: that song's menu."""
    st = page.st or {}
    if label == "Your top song" and st.get("top_songs"):
        queue = stats.ranked_songs(page.boards.get("songs", []))
        return song_menu_link(page.app, queue, st["top_songs"][0]["key"], page.list_name)
    return None


def tracks_of(app, paths: Sequence[str]) -> List[Track]:
    """The library's songs of these paths, in their order, each once (gone files left out)."""
    out: List[Track] = []
    seen = set()
    for p in paths:
        t = app.by_path.get(p)
        if t is not None and p not in seen:
            seen.add(p)
            out.append(t)
    return out


def play_from(app, paths: Sequence[str], path: str, name: str) -> None:
    """A click on a song: play it, with the songs of its list (`paths`) as the queue."""
    tracks = tracks_of(app, list(paths) + [path])
    t = app.by_path.get(path)
    if t is not None:
        app.play_list(tracks, tracks.index(t), name)


def song_menu_at(app, paths: Sequence[str], path: str, name: str, e) -> Optional[tk.Menu]:
    """A right click on a song: its menu (whose Play plays it with its list)."""
    t = app.by_path.get(path)
    if t is None:
        return None
    tracks = tracks_of(app, list(paths) + [path])
    m = app.song_menu(t, list_name=name, rows=tracks, index=tracks.index(t))
    m.tk_popup(e.x_root, e.y_root)
    return m


def artist_page(app, name: str) -> Optional[Tuple[str, str]]:
    """The page an artist's name opens: (grouping, name) of Artists or Album Artists, as
    `PlayerApp.open_artist()` finds it; None when the library has no such artist."""
    for n in [name] + musiclib.split_artists(name, known_artists(app))[:1]:
        for kind in ("artists", "album_artists"):
            g = find_group(app, kind, n)
            if g is not None:
                return kind, g["name"]
    return None


def genre_page(app, name: str) -> Optional[str]:
    g = find_group(app, "genres", name)
    return g["name"] if g is not None else None


def album_of(app, name: str, artist: str, paths: Sequence[str] = ()) -> Optional[dict]:
    """The library's album of a row of Your Stats: the album of one of its songs, else the
    album of that name (and artist)."""
    for p in paths:
        t = app.by_path.get(p)
        a = app.album_of(t) if t is not None else None
        if a is not None:
            return a
    named = [a for a in app.albums if a["title"].strip().casefold() == name.strip().casefold()]
    same = [a for a in named if a["artist"].strip().casefold() == artist.strip().casefold()]
    return (same or named or [None])[0]


def open_group(app, kind: str, name: str) -> None:
    app.show_page("group", kind=kind, group=name)


# ------------------------------------------------------------------ links ---
def add_link(c: tk.Canvas, item: int, hits: Hits, fn: Optional[Callable[[], None]], menus: Optional[Menus] = None,
             menu: Optional[Callable[[object], None]] = None) -> Optional[Box]:
    """Make the drawn text `item` a link: its box runs `fn` (and `menu` on a right click)."""
    if fn is None and menu is None:
        return None
    b = c.bbox(item)
    if not b:
        return None
    box = (b[0] - 2, b[1] - 1, b[2] + 2, b[3] + 1)
    if fn is not None:
        hits.append((box, fn))
    if menu is not None and menus is not None:
        menus.append((box, menu))
    return box


def artist_link(app, name: str) -> Optional[Callable[[], None]]:
    target = artist_page(app, name)
    return (lambda: open_group(app, *target)) if target else None


def album_link(app, name: str, artist: str, paths: Sequence[str] = ()) -> Optional[Callable[[], None]]:
    a = album_of(app, name, artist, paths)
    return (lambda: app.open_album(a)) if a is not None else None


def genre_link(app, name: str) -> Optional[Callable[[], None]]:
    g = genre_page(app, name)
    return (lambda: open_group(app, "genres", g)) if g else None


def song_link(app, paths: Sequence[str], path: str, name: str) -> Optional[Callable[[], None]]:
    return (lambda: play_from(app, paths, path, name)) if path in app.by_path else None


def song_menu_link(app, paths: Sequence[str], path: str, name: str) -> Optional[Callable[[object], None]]:
    return (lambda e: song_menu_at(app, paths, path, name, e)) if path in app.by_path else None


def hit_at(boxes, x: float, y: float):
    for box, fn in boxes:
        if box[0] <= x <= box[2] and box[1] <= y <= box[3]:
            return box, fn
    return None


class LinkArea:
    """The links of a canvas that has no click handlers of its own (Details, Most Played):
    a click runs the link under the pointer, a right click (Button-3; Button-2 on macOS)
    its menu, and the pointer is a hand over a link."""

    def __init__(self, canvas: tk.Canvas):
        self.c = canvas
        self.hits: Hits = []
        self.menus: Menus = []
        canvas.bind("<Button-1>", self._click, add="+")
        canvas.bind("<Motion>", self._motion, add="+")
        canvas.bind("<Leave>", lambda e: canvas.configure(cursor=""), add="+")
        canvas.bind("<Button-3>", self._menu, add="+")
        canvas.bind("<Button-2>", self._menu, add="+")

    def clear(self) -> None:
        self.hits, self.menus = [], []

    def _xy(self, e) -> Tuple[float, float]:
        return self.c.canvasx(e.x), self.c.canvasy(e.y)

    def _click(self, e) -> None:
        hit = hit_at(self.hits, *self._xy(e))
        if hit:
            hit[1]()

    def _menu(self, e) -> None:
        hit = hit_at(self.menus, *self._xy(e))
        if hit:
            hit[1](e)

    def _motion(self, e) -> None:
        x, y = self._xy(e)
        self.c.configure(cursor="hand2" if hit_at(self.hits, x, y) else "")


# ------------------------------------------------------------ leaderboard ---
def segments_width(page: Page, texts: Sequence[str]) -> float:
    """The width of a switch of these (shown) texts."""
    font = page.theme.f("body_b")
    return sum(font.measure(text) + 24 for text in texts) + 4


def draw_segments(page: Page, c: tk.Canvas, x: float, y: float, options: Sequence[Tuple[str, str]], value: str,
                  hits: Hits, pick: Callable[[str], None]) -> float:
    """A segmented switch from (x, y): a rounded panel, the chosen one of `options` ((key,
    shown text)) raised in the page's colour; a click on another calls `pick(key)`.
    Returns its right edge."""
    t = page.theme
    font = t.f("body_b")
    w = segments_width(page, [text for _, text in options])
    round_rect(c, x, y, x + w, y + SEG_H, 9, fill=t.c("lcd"), outline="")
    sx = x + 2
    for key, text in options:
        sw = font.measure(text) + 24
        on = key == value
        box = (sx, y + 2, sx + sw, y + SEG_H - 2)
        if on:
            round_rect(c, *box, 7, fill=t.c("bg"), outline="")
        c.create_text(sx + sw / 2, y + SEG_H / 2, text=text, fill=t.c("text") if on else t.c("text2"), font=font)
        if not on:
            hits.append((box, lambda k=key: pick(k)))
        sx += sw
    return x + w


def draw_switch(page: Page, c: tk.Canvas, x: float, y: float, hits: Hits, on_change: Callable[[], None]) -> float:
    """The leaderboard's switch (Songs, Artists, Albums, Genres) from (x, y); returns its right edge."""
    app = page.app
    return draw_segments(page, c, x, y, [(k, tr(KIND_LABELS[k])) for k in stats.LEADERBOARD_KINDS],
                         app.leaderboard_kind, hits, lambda k: (setattr(app, "leaderboard_kind", k), on_change()))


def _nav_width(page: Page, unit: str, title: str) -> float:
    """The room for the period's name between ‹ and ›: that of a long name of this unit,
    so the buttons stay where they are from one period to the next."""
    font = page.theme.f("body_b")
    sample = {"day": "28 September 2026", "week": "28 September – 4 October 2026", "month": "September 2026",
              "year": "2026"}.get(unit, "")
    sample = tr("{year}", year=sample) if unit == "year" else tr_text(sample)
    return max(font.measure(sample), font.measure(title)) + 16


def draw_controls(page: Page, c: tk.Canvas, x: float, y: float, width: float, hits: Hits,
                  on_change: Callable[[], None], span: bool = False, can_go_back: bool = True) -> float:
    """The leaderboard's controls from (x, y), `width` wide, wrapped onto more lines when
    they do not fit: the switch (Songs…), "Sort by" (Times played / Time listened) and,
    with `span` (Most Played), the period's units (Day … All time) and ‹ name › for the
    first four (‹ only with `can_go_back`, › not at the present).  Returns the bottom."""
    t, app = page.theme, page.app
    groups: List[Tuple[float, Callable[[float, float], object]]] = []
    kinds = [(k, tr(KIND_LABELS[k])) for k in stats.LEADERBOARD_KINDS]
    groups.append((segments_width(page, [text for _, text in kinds]),
                   lambda gx, gy: draw_switch(page, c, gx, gy, hits, on_change)))
    sorts = [(k, tr(SORT_LABELS[k])) for k in stats.LEADERBOARD_SORTS]
    label = tr("Sort by")
    lw = t.f("body").measure(label) + 10

    def draw_sort(gx: float, gy: float) -> None:
        c.create_text(gx, gy + SEG_H / 2, text=label, anchor="w", fill=t.c("text2"), font=t.f("body"))
        draw_segments(page, c, gx + lw, gy, sorts, sort_by(app), hits,
                      lambda k: (setattr(app, "leaderboard_by", k), on_change()))

    groups.append((lw + segments_width(page, [text for _, text in sorts]), draw_sort))
    if span:
        unit, offset = span_of(app)
        units = [(k, tr(UNIT_LABELS[k])) for k in stats.SPAN_UNITS]
        groups.append((segments_width(page, [text for _, text in units]),
                       lambda gx, gy: draw_segments(page, c, gx, gy, units, unit, hits, lambda k: (
                           setattr(app, "leaderboard_span", (k, 0)), on_change()))))
        if unit != "all":
            title = span_text(unit, offset)
            tw = _nav_width(page, unit, title)

            def draw_nav(gx: float, gy: float) -> None:
                mid = gy + SEG_H / 2
                for icon, cx, delta, on in (("back", gx + 15, -1, can_go_back),
                                            ("chevron", gx + 30 + tw + 15, 1, offset < 0)):
                    box = (cx - 15, mid - 15, cx + 15, mid + 15)
                    round_rect(c, *box, 8, fill=t.c("lcd"), outline="")
                    draw_icon(c, icon, cx, mid, 16, t.c("accent") if on else t.c("disabled"))
                    if on:
                        hits.append((box, lambda d=delta: (setattr(app, "leaderboard_span", (unit, offset + d)),
                                                           on_change())))
                c.create_text(gx + 30 + tw / 2, mid, text=title, fill=t.c("text"), font=t.f("body_b"))

            groups.append((30 + tw + 30, draw_nav))
    gx, gap = x, 18
    for w, draw in groups:
        if gx > x and gx + w > x + width:  # no room on this line: the next one
            gx, y = x, y + SEG_H + 10
        draw(gx, y)
        gx += w + gap
    return y + SEG_H + 12


def row_cover(app, kind: str, row: dict) -> Tuple[str, List[str]]:
    """(cover key, paths) of a leaderboard row: the album of its (most played) song."""
    a = album_of(app, row["name"], row["artist"], row["paths"]) if kind == "albums" else None
    if a is None:
        t = next((app.by_path[p] for p in row["paths"] if p in app.by_path), None)
        a = app.album_of(t) if t is not None else None
    if a is not None:
        return a["key"], [x.path for x in a["tracks"]]
    return (row["paths"][0] if row["paths"] else row["key"]), list(row["paths"][:1])


def draw_stat(page: Page, c: tk.Canvas, right: float, mid: float, row: dict, by: str) -> float:
    """A row's plays and minutes, ending at `right`: the number the leaderboard is sorted
    by in the text colour, the other in secondary text (2.5).  Returns the left edge."""
    t = page.theme
    font = t.f("small")
    values = {"plays": plural(row["plays"], "play"), "minutes": f"{row['minutes']:,}"}
    parts = tr("{plays} · {minutes} min").split(" · ")
    if len(parts) != 2:  # a translation without the dot: one text
        sid = c.create_text(right, mid, text=tr("{plays} · {minutes} min", **values), anchor="e",
                            fill=t.c("text2"), font=font)
        box = c.bbox(sid)
        return box[0] if box else right - 140
    x = right
    for i in (1, 0):
        text = parts[i]
        strong = "{minutes}" in text if by == "time" else "{plays}" in text
        for k, v in values.items():
            text = text.replace("{" + k + "}", v)
        for piece, color in ((text, t.c("text") if strong else t.c("text2")),) + (((" · ", t.c("text2")),) if i else ()):
            sid = c.create_text(x, mid, text=piece, anchor="e", fill=color, font=font)
            x -= font.measure(piece)
    return x


def draw_board(page: Page, c: tk.Canvas, x: float, y: float, width: float, kind: str, rows: List[dict],
               limit: int, hits: Hits, menus: Menus, list_name: str, fit: dict, empty: str = "") -> float:
    """Leaderboard rows from (x, y), `width` wide: rank (the top three on a medal), cover,
    name and artist (links), plays and minutes; returns the bottom."""
    t, app = page.theme, page.app
    if not rows:
        c.create_text(x, y + 14, text=empty or tr("No plays yet in this period."), anchor="w", fill=t.c("text2"),
                      font=t.f("body"))
        return y + 40
    queue = stats.ranked_songs(rows)
    right = x + width
    by = sort_by(app)
    for i, r in enumerate(rows[:limit]):
        top, mid = y, y + ROW_H / 2
        if i % 2:
            round_rect(c, x, top + 1, right, top + ROW_H - 1, 8, fill=t.c("zebra"), outline="")
        rank = r["rank"]
        if rank <= 3:
            color = MEDALS[rank - 1]
            c.create_oval(x + 6, mid - 13, x + 32, mid + 13, fill=color, outline="")
            c.create_text(x + 19, mid, text=str(rank), fill="#ffffff", font=t.f("body_b"))
        else:
            c.create_text(x + 19, mid, text=str(rank), fill=t.c("text2"), font=t.f("body_b"))
        key, paths = row_cover(app, kind, r)
        draw_cover(c, t, app.covers.get(key, paths, COVER), x + 42, mid - COVER / 2, COVER)
        room = int(draw_stat(page, c, right - 12, mid, r, by) - (x + 94) - 16)
        name_font = t.f("body_b")
        playing = kind == "songs" and r["key"] == app.current
        if kind == "songs":
            fn, menu = song_link(app, queue, r["key"], list_name), song_menu_link(app, queue, r["key"], list_name)
            name = r["name"]
        elif kind == "artists":
            fn, menu, name = artist_link(app, r["name"]), None, tr_name(r["name"])
        elif kind == "genres":
            fn, menu, name = genre_link(app, r["name"]), None, tr_name(r["name"])
        else:
            fn, menu, name = album_link(app, r["name"], r["artist"], r["paths"]), None, tr_name(r["name"])
        if kind in ("artists", "genres"):
            nid = c.create_text(x + 94, mid, text=fit_text(name_font, name, room, fit), anchor="w",
                                fill=t.c("accent") if playing else t.c("text"), font=name_font)
            add_link(c, nid, hits, fn)
        else:
            nid = c.create_text(x + 94, mid - 9, text=fit_text(name_font, name, room, fit), anchor="w",
                                fill=t.c("accent") if playing else t.c("text"), font=name_font)
            add_link(c, nid, hits, fn, menus, menu)
            aid = c.create_text(x + 94, mid + 10, text=fit_text(t.f("small"), tr_name(r["artist"]), room, fit),
                                anchor="w", fill=t.c("text2"), font=t.f("small"))
            add_link(c, aid, hits, artist_link(app, r["artist"]))
        if kind == "songs" and menu is not None:  # the whole row opens the song menu on a right click
            menus.append(((x, top, right, top + ROW_H), menu))
        y += ROW_H
    return y + 8


def section_heading(page: Page, c: tk.Canvas, x: float, y: float, width: float, hits: Hits,
                    on_change: Callable[[], None], see_all: bool, can_go_back: bool = True) -> float:
    """"Leaderboard" and See All (Your Stats), then the controls: the switch and Sort by,
    and on the Most Played page (`see_all` False) the period control."""
    t = page.theme
    if see_all:
        c.create_text(x, y + 14, text=tr("Leaderboard"), anchor="w", fill=t.c("text"), font=t.f("h2"))
        sid = c.create_text(x + width, y + 14, text=tr("See All"), anchor="e", fill=t.c("accent"),
                            font=t.f("body_b"))
        add_link(c, sid, hits, lambda: open_most_played(page.app))
        y += 36
    return draw_controls(page, c, x, y, width, hits, on_change, span=not see_all, can_go_back=can_go_back)


# ------------------------------------------------------------- Most Played ---
class MostPlayedPage(Page):
    """The leaderboard as a page of its own (2.4): Play and Shuffle (the ranked songs in
    rank order), the switch, Sort by, the period control (2.5: Day, Week, Month, Year,
    All time, with ‹ and ›) and up to 100 rows of that period."""

    list_name = N_("Most Played")

    def __init__(self, app):
        super().__init__(app)
        top, self.title_l, self.sub = self.heading(self, tr("Most Played"), back=True)
        top.pack(fill="x", padx=28, pady=(22, 6))
        from .player_collections import ScrollCanvas  # noqa: PLC0415 (that module imports this one)

        self.area = ScrollCanvas(self, self, self._draw)
        self.area.pack(fill="both", expand=True)
        self.links = LinkArea(self.area.c)
        self.rows: List[dict] = []
        self.older = False  # the history holds plays before the period shown (‹ leads somewhere)
        self._fit: dict = {}
        app.covers.listeners.append(lambda key, size: size == COVER and self.winfo_exists()
                                    and app.page_name == "most_played" and self.area.redraw())

    def show(self, **kw) -> None:
        self.refresh()

    def span(self) -> Tuple[float, Optional[float]]:
        """[since, until) of the period shown."""
        return stats.span_range(*span_of(self.app), time.time())

    def refresh(self) -> None:
        app = self.app
        unit, offset = span_of(app)
        since, until = self.span()
        self.rows = stats.leaderboard(app.plays(since, until), app.leaderboard_kind, by=sort_by(app))
        first = first_play(app) if unit != "all" else None
        self.older = first is not None and first < since
        self.sub.configure(text=span_text(unit, offset))
        self.area.redraw()

    def step(self, delta: int) -> None:
        """The period before (-1) or after (+1) the one shown, never past the present."""
        unit, offset = span_of(self.app)
        if unit != "all" and offset + delta <= 0 and (delta > 0 or self.older):
            self.app.leaderboard_span = (unit, offset + delta)
            self.refresh()

    def song_changed(self) -> None:
        if self.app.leaderboard_kind == "songs":
            self.area.redraw()

    def songs(self) -> List[Track]:
        """What Play and Shuffle play: the ranked songs in rank order."""
        return tracks_of(self.app, stats.ranked_songs(self.rows))

    def _draw(self) -> float:
        c = self.area.c
        W = c.winfo_width()
        self.links.clear()
        y = section_heading(self, c, 28, 6, W - 56, self.links.hits, self.refresh, see_all=False,
                            can_go_back=self.older)
        return draw_board(self, c, 28, y, W - 56, self.app.leaderboard_kind, self.rows, stats.TOP,
                          self.links.hits, self.links.menus, self.list_name, self._fit,
                          empty=tr("Nothing was played in this period.")) + 20


__all__ = ["MostPlayedPage", "LinkArea", "draw_board", "section_heading", "period_plays", "boards", "add_link",
           "artist_link", "album_link", "genre_link", "song_link", "song_menu_link", "play_from", "tracks_of",
           "MEDALS", "SECTION_ROWS", "sort_by", "span_of", "span_text", "open_most_played", "card_target",
           "card_menu", "draw_controls", "draw_segments"]
