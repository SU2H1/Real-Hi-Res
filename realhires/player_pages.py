"""The pages of the 2.0 player's content area: Recently Added, Albums, an album,
Artists, Songs, Genres, Liked Songs, Search, Now Playing and Settings (the
Quality check page is the 1.x scanner, `gui.App`)."""

from __future__ import annotations

import os
import tkinter as tk
from tkinter import ttk
from typing import TYPE_CHECKING, Any, Callable, Dict, List, Optional

from . import APP_NAME, embed
from . import lyrics as lyrics_mod
from . import musiclib
from . import playerstate
from . import settings as settings_mod
from .musiclib import Track
from .player_theme import DEFAULT_FONT_LABEL, IconButton, PillButton, Theme, fit_text, font_families, round_rect
from .player_widgets import AlbumGrid, Column, SongList, bind_wheel, draw_cover, fmt_time, show_scrollbar

if TYPE_CHECKING:  # pragma: no cover
    from .player_gui import PlayerApp


# ------------------------------------------------------------- helpers ---
def label(parent, theme: Theme, text: str = "", font: str = "body", fg: str = "text", bg: str = "bg",
          **kw) -> tk.Label:
    w = tk.Label(parent, text=text, font=theme.f(font), anchor=kw.pop("anchor", "w"), bd=0, **kw)
    return theme.reg(w, fg=fg, bg=bg)


def frame(parent, theme: Theme, bg: str = "bg", **kw) -> tk.Frame:
    return theme.reg(tk.Frame(parent, bd=0, highlightthickness=0, **kw), bg=bg)


def track_value(t: Track, key: str) -> str:
    if key == "title":
        return t.display_title
    if key == "artist":
        return "Not in the library" if getattr(t, "missing", False) else t.display_artist
    if key == "album":
        return t.display_album
    if key == "genre":
        return t.genre
    if key == "time":
        return fmt_time(t.duration)
    if key == "quality":
        return musiclib.quality(t)
    if key == "num":
        return str(t.track) if t.track else ""
    if key == "year":
        return t.year
    if key == "composer":
        return t.composer
    return ""


def track_sort(t: Track, key: str):
    if key == "time":
        return t.duration
    if key == "num":
        return (t.disc or 9999, t.track or 9999, t.display_title.casefold())
    return (track_value(t, key).casefold(), t.display_title.casefold(), t.path)


def album_order(tracks: List[Track]) -> List[Track]:
    return [t for a in musiclib.albums(tracks) for t in a["tracks"]]


def plural(n: int, one: str, many: str = "") -> str:
    return f"{n:,} {one if n == 1 else (many or one + 's')}"


SONG_COLUMNS = [Column("title", "Title", 300, True), Column("artist", "Artist", 200, True),
                Column("album", "Album", 200, True), Column("genre", "Genre", 110),
                Column("time", "Time", 52, anchor="e"), Column("quality", "Quality", 118),
                Column("like", "", 30, kind="heart")]


class Page(tk.Frame):
    """One page of the content area."""

    list_name = ""

    def __init__(self, app: "PlayerApp"):
        # a requested size: a gridded frame that asks for 0x0 (only placed children) is never shown
        super().__init__(app.content, bd=0, highlightthickness=0, width=400, height=300)
        self.app, self.theme = app, app.theme
        self.theme.reg(self, bg="bg")

    def show(self, **kw) -> None:
        pass

    def refresh(self) -> None:
        """The library changed."""

    def songs(self) -> List[Track]:
        """What the Play button of the playback bar plays when nothing plays."""
        return album_order(self.app.tracks)

    def likes_changed(self) -> None:
        pass

    def song_changed(self) -> None:
        pass

    def heading(self, parent, title: str, buttons: bool = True, back: bool = False):
        """The page title, a grey line under it, and Play / Shuffle."""
        t = self.theme
        top = frame(parent, t)
        if back:
            IconButton(top, t, "back", command=self.app.go_back, size=30, icon_size=16,
                       tooltip=lambda: "Back").pack(side="left", padx=(0, 6))
        box = frame(top, t)
        box.pack(side="left", fill="x", expand=True)
        title_l = label(box, t, title, "title")
        title_l.pack(anchor="w")
        sub = label(box, t, "", "body", fg="text2")
        sub.pack(anchor="w")
        if buttons:
            PillButton(top, t, "Shuffle", icon="shuffle", command=lambda: self._play(True)).pack(side="right")
            PillButton(top, t, "Play", icon="play", command=lambda: self._play(False)).pack(side="right",
                                                                                            padx=(0, 10))
        return top, title_l, sub

    def _play(self, shuffle: bool) -> None:
        songs = self.songs()
        if songs:
            self.app.play_list(songs, 0, self.list_name, shuffle=shuffle)


def song_table(page: Page, parent, columns=None, value=None, extra_menu=None, sort_value=None, **kw) -> SongList:
    """A song table with likes, the playing mark and the song menu (right click);
    `extra_menu(index)` gives more (label, command) entries, e.g. a playlist's."""
    app = page.app
    table: SongList

    def context(i: int, e) -> None:
        app.song_menu(table.rows[i], extra_menu(i) if extra_menu else (), list_name=page.list_name,
                      rows=table.rows, index=i).tk_popup(e.x_root, e.y_root)

    table = SongList(parent, page.theme, columns or SONG_COLUMNS, value or track_value, sort_value or track_sort,
                     on_heart=lambda r: app.toggle_like(r.path), is_liked=lambda r: app.is_liked(r.path),
                     is_current=lambda r: r.path == app.current, on_context=context,
                     is_dim=lambda r: getattr(r, "missing", False), **kw)
    return table


# --------------------------------------------------------------- albums ---
class AlbumsPage(Page):
    """Album cards: all albums by title, or (Recently Added) the newest first."""

    def __init__(self, app, title: str, recent: bool = False):
        super().__init__(app)
        self.title, self.recent, self.list_name = title, recent, title
        t = self.theme
        head, self.title_l, self.sub = self.heading(self, title, buttons=False)
        head.pack(fill="x", padx=28, pady=(22, 6))
        self.empty = frame(self, t)
        self.empty_text = label(self.empty, t, "", "h2", fg="text2", anchor="center")
        self.empty_text.pack(pady=(80, 6))
        self.empty_sub = label(self.empty, t, "", "body", fg="text2", anchor="center", justify="center",
                               wraplength=460)
        self.empty_sub.pack(pady=(0, 14))
        self.choose = PillButton(self.empty, t, "Choose a music folder…", command=app.choose_music_folder)
        self.choose.pack()
        self.grid_w = AlbumGrid(self, t, app.covers, app.open_album)
        self.albums: List[dict] = []
        self.refresh()

    def refresh(self) -> None:
        albums = list(self.app.albums)
        if self.recent:
            albums.sort(key=lambda a: max((tr.mtime_ns for tr in a["tracks"]), default=0), reverse=True)
        self.albums = albums
        n = len(albums)
        self.sub.configure(text=plural(n, "album") if n else "")
        if albums:
            self.empty.pack_forget()
            self.grid_w.pack(fill="both", expand=True)
            self.grid_w.set_albums(albums, keep=True)
            return
        self.grid_w.pack_forget()
        self.empty.pack(fill="both", expand=True)
        if not self.app.cfg.library_folders:
            self.empty_text.configure(text="Add your music folder in Settings to start")
            self.empty_sub.configure(text="Universal Music Player lists every song in your music folders, plays them, and "
                                          "checks them for fake hi-res and fake lossless files.")
            self.choose.pack()
        else:
            busy = self.app.library_busy()
            self.empty_text.configure(text="Reading your music library…" if busy else "No songs found")
            self.empty_sub.configure(text="" if busy else "Your music folders hold no audio files with the "
                                                         "extensions set in Settings → Scanning.")
            self.choose.pack_forget()

    def songs(self) -> List[Track]:
        return [tr for a in self.albums for tr in a["tracks"]]


class AlbumPage(Page):
    COVER = 220

    def __init__(self, app):
        super().__init__(app)
        t = self.theme
        self.album: Optional[dict] = None
        top = frame(self, t)
        top.pack(fill="x", padx=28, pady=(18, 10))
        IconButton(top, t, "back", command=app.go_back, size=30, icon_size=16,
                   tooltip=lambda: "Back").pack(side="left", anchor="n", padx=(0, 6))
        self.cover = tk.Canvas(top, width=self.COVER, height=self.COVER, highlightthickness=0, bd=0)
        t.reg(self.cover, bg="bg")
        self.cover.pack(side="left")
        info = frame(top, t)
        info.pack(side="left", fill="both", expand=True, padx=(24, 0))
        self.title_l = label(info, t, "", "big", wraplength=600, justify="left")
        self.title_l.pack(anchor="w", pady=(60, 2))
        self.artist_l = label(info, t, "", "h2", fg="accent", cursor="hand2")
        self.artist_l.pack(anchor="w")
        self.artist_l.bind("<Button-1>", lambda e: self.album and app.open_artist(self.album["artist"]))
        self.meta_l = label(info, t, "", "body", fg="text2")
        self.meta_l.pack(anchor="w", pady=(2, 14))
        btns = frame(info, t)
        btns.pack(anchor="w")
        PillButton(btns, t, "Play", icon="play", command=lambda: self._play(False)).pack(side="left")
        PillButton(btns, t, "Shuffle", icon="shuffle", command=lambda: self._play(True)).pack(side="left",
                                                                                              padx=(10, 0))
        self.edit_b = PillButton(btns, t, "Album Properties…", kind="plain", command=self.edit_info)
        self.edit_b.pack(side="left", padx=(10, 0))
        cols = [Column("num", "", 34, anchor="e", kind="num"), Column("title", "", 400, True),
                Column("like", "", 30, kind="heart"), Column("time", "", 52, anchor="e")]
        self.table = song_table(self, self, cols, header=False, sortable=False,
                                on_activate=lambda i: self.app.play_list(self.table.rows, i, self.list_name))
        self.table.pack(fill="both", expand=True, padx=(16, 16))
        app.covers.listeners.append(self._cover_ready)
        t.on_change(self._draw_cover)

    def show(self, album: Optional[dict] = None, **kw) -> None:
        if album is not None:
            self.album = album
        self._fill()

    def refresh(self) -> None:
        if self.album is not None:
            fresh = self.app.album_by_key.get(self.album["key"])
            if fresh is None and self.album["tracks"]:  # its title or album artist was edited
                first = self.app.track(self.album["tracks"][0].path)
                fresh = self.app.album_of(first) if first is not None else None
            if fresh is not None:
                self.album = fresh
        self._fill()

    def edit_info(self):
        """Album Properties…: the Properties window for every song of the album."""
        if self.album is not None:
            return self.app.edit_album_info(self.album)
        return None

    def _fill(self) -> None:
        a = self.album
        if a is None:
            return
        self.list_name = a["title"]
        self.title_l.configure(text=a["title"])
        self.artist_l.configure(text=a["artist"])
        meta = " · ".join(x for x in (a["genre"], a["year"]) if x)
        self.meta_l.configure(text=meta)
        self.table.set_rows(a["tracks"])
        minutes = int(round(sum(tr.duration for tr in a["tracks"]) / 60))
        self.table.footer = f"{plural(len(a['tracks']), 'song')}, {plural(minutes, 'minute')}"
        self.table.redraw()
        self._draw_cover()

    def _draw_cover(self) -> None:
        if self.album is None:
            return
        c = self.cover
        c.delete("all")
        img = self.app.covers.get(self.album["key"], [tr.path for tr in self.album["tracks"]], self.COVER)
        draw_cover(c, self.theme, img, 0, 0, self.COVER)

    def _cover_ready(self, key: str, size: int) -> None:
        if self.album is not None and key == self.album["key"] and size == self.COVER and self.winfo_exists():
            self._draw_cover()

    def songs(self) -> List[Track]:
        return list(self.album["tracks"]) if self.album else []

    def likes_changed(self) -> None:
        self.table.redraw()

    def song_changed(self) -> None:
        self.table.redraw()


# ------------------------------------------------------------ groupings ---
GROUPINGS = {  # page: (title, musiclib grouping, noun, what the right side shows)
    "artists": ("Artists", musiclib.artists, "artist", "albums"),
    "album_artists": ("Album Artists", musiclib.album_artists, "album artist", "albums"),
    "genres": ("Genres", musiclib.genres, "genre", "songs"),
    "years": ("Years", musiclib.years, "year", "songs"),
    "composers": ("Composers", musiclib.composers, "composer", "songs"),
}


class GroupPage(Page):
    """Artists, Album Artists, Genres, Years, Composers: the names on the left with
    their song counts, the chosen one's albums (cards) or songs (a table) on the right."""

    def __init__(self, app, kind: str):
        super().__init__(app)
        t = self.theme
        self.kind = kind
        self.title, self.group_fn, self.noun, self.shows = GROUPINGS[kind]
        self.list_name = self.title
        head, self.title_l, self.sub = self.heading(self, self.title, buttons=False)
        head.pack(fill="x", padx=28, pady=(22, 8))
        body = frame(self, t)
        body.pack(fill="both", expand=True)
        self.left = SongList(body, t, [Column("name", "", 200, True), Column("count", "", 44, anchor="e")],
                             lambda r, k: r["name"] if k == "name" else f"{len(r['tracks']):,}", header=False,
                             zebra=False, on_select=self._chosen, on_activate=self._activate, row_height=34)
        self.left.configure(width=270)
        self.left.pack(side="left", fill="y", padx=(12, 0))
        self.left.pack_propagate(False)
        frame(body, t, bg="sep", width=1).pack(side="left", fill="y", padx=(8, 0))
        self.right = frame(body, t)
        self.right.pack(side="left", fill="both", expand=True)
        rh, self.r_title, self.r_sub = self.heading(self.right, "", buttons=True)
        self.r_title.configure(font=t.f("h1"))
        rh.pack(fill="x", padx=24, pady=(6, 8))
        self.items: List[dict] = []
        self.chosen: Optional[dict] = None
        if self.shows == "albums":
            self.grid_w = AlbumGrid(self.right, t, app.covers, app.open_album)
            self.grid_w.pack(fill="both", expand=True)
        else:
            self.table = song_table(self, self.right, on_activate=lambda i: app.play_list(
                self.table.rows, i, self.list_name))
            self.table.pack(fill="both", expand=True, padx=(8, 12))
        self.refresh()

    def refresh(self) -> None:
        self.items = self.group_fn(self.app.tracks)
        self.sub.configure(text=plural(len(self.items), self.noun) if self.items else "")
        names = [i["name"] for i in self.items]
        keep = self.chosen["name"] if self.chosen else None
        self.left.set_rows(self.items, keep=True)
        idx = names.index(keep) if keep in names else (0 if self.items else None)
        self.left.selected = idx
        self.left.redraw()
        self._chosen(idx)

    def _chosen(self, i: Optional[int]) -> None:
        self.chosen = self.items[i] if i is not None and i < len(self.items) else None
        item = self.chosen
        self.r_title.configure(text=item["name"] if item else "")
        self.list_name = item["name"] if item else self.title
        if self.shows == "albums":
            albums = musiclib.albums(item["tracks"]) if item else []
            self.r_sub.configure(text=f"{plural(len(albums), 'album')}, {plural(len(item['tracks']), 'song')}"
                                 if item else "")
            self.grid_w.set_albums(albums)
        else:
            self.r_sub.configure(text=plural(len(item["tracks"]), "song") if item else "")
            self.table.set_rows(item["tracks"] if item else [])

    def _activate(self, i: int) -> None:
        self._chosen(i)
        self._play(False)

    def choose(self, name: str) -> None:
        for i, item in enumerate(self.items):
            if item["name"].casefold() == name.casefold():
                self.left.select(i)
                return

    def songs(self) -> List[Track]:
        if not self.chosen:
            return []
        return album_order(self.chosen["tracks"]) if self.shows == "albums" else list(self.table.rows)

    def likes_changed(self) -> None:
        if self.shows == "songs":
            self.table.redraw()

    def song_changed(self) -> None:
        if self.shows == "songs":
            self.table.redraw()


# ---------------------------------------------------------------- songs ---
class SongsPage(Page):
    """All Songs, Liked Songs or Disliked Songs."""

    EMPTY = {"all": "No songs yet. Add your music folder in Settings.",
             "liked": "Songs you like appear here. Click the heart of a song to like it.",
             "disliked": "Songs you dislike appear here, and shuffles leave them out. Use the thumbs-down "
                         "button or a song's menu to dislike it."}

    def __init__(self, app, title: str, which: str = "all"):
        super().__init__(app)
        self.title, self.which, self.list_name = title, which, title
        head, self.title_l, self.sub = self.heading(self, title)
        head.pack(fill="x", padx=28, pady=(22, 8))
        self.table = song_table(self, self, on_activate=lambda i: app.play_list(self.table.rows, i, self.list_name),
                                empty_text=self.EMPTY[which])
        self.table.pack(fill="both", expand=True, padx=12, pady=(0, 8))
        self.refresh()

    def rows(self) -> List[Track]:
        tracks = musiclib.sort_songs(self.app.tracks)
        if self.which != "all":
            wanted = set(self.app.state.liked if self.which == "liked" else self.app.state.disliked)
            tracks = [t for t in tracks if t.path in wanted]
        return tracks

    def refresh(self) -> None:
        rows = self.rows()
        self.table.set_rows(rows, keep=True)
        minutes = int(round(sum(t.duration for t in rows) / 60))
        self.sub.configure(text=f"{plural(len(rows), 'song')}, {plural(minutes, 'minute')}" if rows else "")

    def songs(self) -> List[Track]:
        return list(self.table.rows)

    def likes_changed(self) -> None:
        if self.which != "all":
            self.refresh()
        else:
            self.table.redraw()

    def song_changed(self) -> None:
        self.table.redraw()


# --------------------------------------------------------------- search ---
class SearchPage(Page):
    list_name = "Search results"

    def __init__(self, app):
        super().__init__(app)
        t = self.theme
        head, self.title_l, self.sub = self.heading(self, "Search", buttons=False)
        head.pack(fill="x", padx=28, pady=(22, 4))
        self.alb_head = label(self, t, "Albums", "h2")
        self.alb_head.pack(anchor="w", padx=28, pady=(6, 0))
        self.grid_box = frame(self, t, height=AlbumGrid.COVER + 84)  # one row of cards
        self.grid_box.pack(fill="x")
        self.grid_box.pack_propagate(False)
        self.grid_w = AlbumGrid(self.grid_box, t, app.covers, app.open_album)
        self.grid_w.pack(fill="both", expand=True)
        self.song_head = label(self, t, "Songs", "h2")
        self.song_head.pack(anchor="w", padx=28, pady=(4, 2))
        self.table = song_table(self, self, on_activate=lambda i: app.play_list(self.table.rows, i, self.list_name))
        self.table.pack(fill="both", expand=True, padx=12, pady=(0, 8))
        self.text = ""

    def show(self, text: str = "", **kw) -> None:
        self.text = text
        self.refresh()

    def refresh(self) -> None:
        q = self.text.strip().casefold()
        tracks = [t for t in self.app.tracks if q and any(q in x.casefold() for x in (
            t.display_title, t.artist, t.album_artist, t.display_album, t.genre))]
        albums = [a for a in self.app.albums if q and any(q in x.casefold() for x in (
            a["title"], a["artist"], a["genre"]))]
        self.title_l.configure(text=f"Results for “{self.text.strip()}”")
        self.sub.configure(text=f"{plural(len(albums), 'album')}, {plural(len(tracks), 'song')}" if (
            albums or tracks) else "Nothing in your library matches this.")
        self.grid_w.set_albums(albums)
        self.grid_box.configure(height=AlbumGrid.COVER + 84 if albums else 1)
        self.table.set_rows(musiclib.sort_songs(tracks))

    def songs(self) -> List[Track]:
        return list(self.table.rows)

    def likes_changed(self) -> None:
        self.table.redraw()

    def song_changed(self) -> None:
        self.table.redraw()


# ---------------------------------------------------------- now playing ---
NP_TABS = (("lyrics", "Lyrics"), ("upnext", "Up Next"), ("details", "Details"))


def fit_pair(font, a: str, sep: str, b: str, width: int, cache: Optional[dict] = None) -> tuple:
    """`a`, `b` cut with "…" so that "a sep b" fits in `width` pixels; the shorter keeps
    its room and the longer gets the rest (each at least half)."""
    aw, sw, bw = font.measure(a), font.measure(sep), font.measure(b)
    if aw + sw + bw <= width:
        return a, b
    room = max(0, width - sw)
    if aw <= room / 2:
        return a, fit_text(font, b, int(room - aw), cache)
    if bw <= room / 2:
        return fit_text(font, a, int(room - bw), cache), b
    return fit_text(font, a, int(room / 2), cache), fit_text(font, b, int(room / 2), cache)


def artist_line(font, artists: List[str], album_link: Optional[str], album: str, width: int,
                cache: Optional[dict] = None) -> List[tuple]:
    """"Artist, Artist — Album" in pieces (text, link, artist name): each artist its own
    link ("artist"), the album name a link ("album_name") when `album_link` is given, the
    separators plain; cut with "…" to fit in `width` pixels (the shorter part keeps its room)."""
    sep = " — "
    joined, album_shown = fit_pair(font, ", ".join(artists), sep, album, width, cache)
    budget = font.measure(joined)
    pieces: List[tuple] = []
    used = 0
    for i, name in enumerate(artists):
        for text, what in (((", ", ""),) if i else ()) + ((name, "artist"),):
            w = font.measure(text)
            if used + w <= budget:
                pieces.append((text, what, name if what else None))
                used += w
                continue
            cut = fit_text(font, text, int(budget - used), cache) if what else ""
            if cut:
                pieces.append((cut, what, name))
            elif pieces and pieces[-1][1]:  # no room for ", ": the last name ends with "…"
                last = pieces[-1]
                pieces[-1] = (fit_text(font, last[0] + "…", font.measure(last[0]), cache), last[1], last[2])
            break
        else:
            continue
        break
    pieces.append((sep, "", None))
    pieces.append((album_shown, "album_name" if album_link is not None else "", None))
    return pieces


def format_line(t: Track) -> str:
    """Details' format line: "FLAC · 24-bit/96 kHz · 2,304 kbps" (lossy: "MP3 · 44.1 kHz ·
    320 kbps"); parts that are unknown are left out."""
    parts = [t.codec.upper()] if t.codec else []
    khz = f"{t.sample_rate / 1000:g} kHz" if t.sample_rate else ""
    if khz:
        if t.is_lossy or not t.bits:
            parts.append(khz)
        else:
            parts.append(f"{t.bits}-bit{' float' if t.is_float else ''}/{khz}")
    if t.bit_rate:
        parts.append(f"{round(t.bit_rate / 1000):,} kbps")
    return " · ".join(parts) or "—"


def song_details(t: Track, album_tracks: int, comment: str, verdict: Optional[tuple],
                 artists: Optional[List[str]] = None) -> List[tuple]:
    """Now Playing's Details: (label, value) rows.  `verdict` is (verdict label, summary)
    of a saved Quality check result, or None; `artists` the artists of the artist field
    (`musiclib.split_artists()`), shown as ", "-joined links."""
    dash = "—"
    if t.track and album_tracks >= t.track and album_tracks > 1:
        track_no = f"{t.track} of {album_tracks}"
    else:
        track_no = str(t.track) if t.track else dash
    artist = ", ".join(artists) if artists and t.artist else t.artist
    rows = [("Title", t.display_title), ("Artist", artist or dash), ("Album", t.album or dash),
            ("Album artist", t.album_artist or dash), ("Composer", t.composer or dash), ("Genre", t.genre or dash),
            ("Year", t.year or dash), ("Track", track_no), ("Disc", str(t.disc) if t.disc else dash),
            ("Duration", fmt_time(t.duration)), ("Format", format_line(t)), ("File", t.path)]
    if comment.strip():
        rows.append(("Comment", comment.strip()))
    if verdict is None:
        rows.append(("Quality check", "Not checked yet"))
    else:
        rows.append(("Quality check", verdict[0]))
        if verdict[1]:
            rows.append(("Why", verdict[1]))
    return rows


def read_comment(path: str) -> str:
    """The comment tag of a song ("" when it has none, or the file cannot be read); ffprobe,
    so call it off the window's thread."""
    import json
    import subprocess

    from .probe import _no_window, tool

    try:
        out = subprocess.run([tool("ffprobe"), "-v", "error", "-show_entries", "format_tags:stream_tags",
                              "-of", "json", path], capture_output=True, timeout=20, **_no_window()).stdout
        data = json.loads(out.decode("utf-8", "replace") or "{}")
    except (OSError, ValueError, subprocess.SubprocessError):
        return ""
    for src in [data.get("format", {})] + list(data.get("streams", [])):
        for k, v in (src.get("tags") or {}).items():
            if k.strip().casefold() == "comment" and isinstance(v, str) and v.strip():
                return v.strip()
    return ""


class Segmented(tk.Canvas):
    """A segmented switch (Now Playing's Lyrics | Up Next | Details): a rounded panel in the
    LCD colour, the chosen segment raised in the page colour."""

    H = 30

    def __init__(self, master, theme: Theme, options, command: Callable[[str], None], value: str = ""):
        super().__init__(master, height=self.H, width=300, highlightthickness=0, bd=0)
        self.theme, self.options, self.command = theme, list(options), command
        self.value = value or self.options[0][0]
        self.hover: Optional[str] = None
        self.boxes: List[tuple] = []
        theme.reg(self, bg="bg")
        theme.on_change(self.redraw, self)
        self.bind("<Motion>", lambda e: self._hov(self._at(e.x)))
        self.bind("<Leave>", lambda e: self._hov(None))
        self.bind("<ButtonRelease-1>", self._click)
        self.redraw()

    def _at(self, x: float) -> Optional[str]:
        return next((k for k, x0, x1 in self.boxes if x0 <= x <= x1), None)

    def _hov(self, key: Optional[str]) -> None:
        if key != self.hover:
            self.hover = key
            self.configure(cursor="hand2" if key and key != self.value else "")
            self.redraw()

    def _click(self, e) -> None:
        key = self._at(e.x)
        if key is not None:
            self.choose(key)

    def choose(self, key: str) -> None:
        self.set(key)
        self.command(key)

    def set(self, key: str) -> None:
        if key != self.value:
            self.value = key
            self.redraw()

    def redraw(self) -> None:
        t = self.theme
        font = t.f("body_b")
        seg = max(font.measure(text) for _, text in self.options) + 30
        w = seg * len(self.options) + 4
        self.configure(width=w)
        self.delete("all")
        round_rect(self, 0, 0, w, self.H, 9, fill=t.c("lcd"), outline="")
        self.boxes = []
        for i, (key, text) in enumerate(self.options):
            x0, x1 = 2 + i * seg, 2 + (i + 1) * seg
            self.boxes.append((key, x0, x1))
            if key == self.value:
                round_rect(self, x0, 2, x1, self.H - 2, 7, fill=t.c("bg"), outline="")
            elif key == self.hover:
                round_rect(self, x0, 2, x1, self.H - 2, 7, fill=t.c("hover"), outline="")
            self.create_text((x0 + x1) / 2, self.H / 2, text=text, font=font,
                             fill=t.c("text") if key == self.value else t.c("text2"))


class LyricsView(tk.Canvas):
    """Synced lyrics: every line, the current one bold in the text colour and the others
    grey, kept about a third from the top; a click on a line seeks there.  When the user
    scrolls the current line out of view, following stops and `on_away(True)` shows the
    "Back to the current line" button until it is pressed (`back_to_line()`) or the user
    scrolls the line back into view (2.1).  Plain lyrics: the text, scrollable.  Sizes
    from the theme's lyrics fonts (A− / A+)."""

    PAD = 40
    GAP = 16

    def __init__(self, master, app: "PlayerApp", on_away: Optional[Callable[[bool], None]] = None):
        super().__init__(master, highlightthickness=0, bd=0)
        self.app, self.theme = app, app.theme
        self.on_away = on_away
        self.theme.reg(self, bg="bg")
        self.kind = ""
        self.entries: List[tuple] = []
        self.plain = ""
        self.items: List[int] = []
        self.tops: List[float] = []
        self.heights: List[float] = []
        self.current = -1
        self.offset = 0.0
        self.target = 0.0
        self.total = 1.0
        self.away = False      # the user scrolled the current line out of view
        self.followed = -2     # the line scrolled to last (-2: none yet)
        self._anim: Optional[str] = None
        self.bind("<Configure>", lambda e: self.layout())
        self.bind("<Button-1>", self._click)
        self.bind("<Motion>", self._motion)
        bind_wheel(self, self._wheel)
        self.theme.on_change(self.layout, self)

    def set_lyrics(self, kind: str, text: str) -> None:
        self.kind = kind
        self.entries = embed.lrc_entries(text) if kind == lyrics_mod.SYNCED else []
        if kind == lyrics_mod.SYNCED and not self.entries:
            self.kind, self.plain = lyrics_mod.PLAIN, lyrics_mod.strip_timestamps(text)
        else:
            self.plain = text if kind == lyrics_mod.PLAIN else ""
        self.current, self.offset, self.target, self.followed = -1, 0.0, 0.0, -2
        self._set_away(False)
        self.layout()

    def _set_away(self, away: bool) -> None:
        if away != self.away:
            self.away = away
            if self.on_away is not None:
                self.on_away(away)

    def layout(self) -> None:
        t = self.theme
        self.delete("all")
        self.items, self.tops, self.heights = [], [], []
        W, H = self.winfo_width(), self.winfo_height()
        if W <= 1:
            return
        width = max(100, W - 2 * self.PAD)
        y = self.PAD / 2
        if self.kind == lyrics_mod.SYNCED:
            bold, normal = t.f("lyrics_b"), t.f("lyrics")
            for i, (_, line) in enumerate(self.entries):
                item = self.create_text(self.PAD, y, text=line or " ", anchor="nw", width=width, font=bold,
                                        fill=t.c("text2"), tags=("line",))
                box = self.bbox(item)
                h = (box[3] - box[1]) if box else bold.metrics("linespace")
                if i != self.current:
                    self.itemconfigure(item, font=normal)
                else:
                    self.itemconfigure(item, fill=t.c("text"))
                self.items.append(item)
                self.tops.append(y)
                self.heights.append(h)
                y += h + self.GAP
        else:
            text = self.plain if self.kind == lyrics_mod.PLAIN else "This song is instrumental."
            item = self.create_text(self.PAD, y, text=text, anchor="nw", width=width, font=t.f("plain"),
                                    fill=t.c("text") if self.kind == lyrics_mod.PLAIN else t.c("text2"))
            box = self.bbox(item)
            y += (box[3] - box[1]) if box else 0
        self.total = y + self.PAD + (H * 2 / 3 if self.kind == lyrics_mod.SYNCED else 0)
        self.configure(scrollregion=(0, 0, W, max(self.total, H)))
        if self.kind == lyrics_mod.SYNCED and not self.away:
            self._scroll_to(self._line_target())  # the current line stays where it was
            self.target = self.offset
        else:
            self._scroll_to(self.offset)

    def _scroll_to(self, offset: float) -> None:
        H = self.winfo_height()
        self.offset = min(max(offset, 0.0), max(0.0, self.total - H))
        self.yview_moveto(self.offset / max(self.total, H, 1))

    def _line_target(self) -> float:
        """The offset that shows the current line a third from the top."""
        if 0 <= self.current < len(self.tops):
            return self.tops[self.current] + self.heights[self.current] / 2 - self.winfo_height() / 3
        return 0.0

    def line_in_view(self) -> bool:
        """The current line (the first one before it starts) is wholly in view."""
        i = self.current if 0 <= self.current < len(self.tops) else 0
        if not self.tops:
            return True
        return self.offset - 1 <= self.tops[i] and self.tops[i] + self.heights[i] <= self.offset + \
            self.winfo_height() + 1

    def update_position(self, seconds: float) -> None:
        if self.kind != lyrics_mod.SYNCED or not self.entries:
            return
        ms = seconds * 1000.0
        idx = -1
        for i, (t_ms, _) in enumerate(self.entries):
            if t_ms <= ms:
                idx = i
            else:
                break
        if idx != self.current:
            t = self.theme
            if 0 <= self.current < len(self.items):
                self.itemconfigure(self.items[self.current], font=t.f("lyrics"), fill=t.c("text2"))
            self.current = idx
            if 0 <= idx < len(self.items):
                self.itemconfigure(self.items[idx], font=t.f("lyrics_b"), fill=t.c("text"))
        if self.away:
            if self.line_in_view():  # the user scrolled the current line back into view
                self.followed = self.current
                self._set_away(False)
            return
        if self.current != self.followed:  # a new line: scroll it smoothly to a third from the top
            self.followed = self.current
            self.target = self._line_target()
            if self._anim is None and abs(self.target - self.offset) > 0.5:
                self._animate()

    def _animate(self) -> None:
        self._anim = None
        if self.away:
            return
        diff = self.target - self.offset
        if abs(diff) < 0.6:
            self._scroll_to(self.target)
            return
        self._scroll_to(self.offset + diff * 0.18)
        self._anim = self.after(16, self._animate)

    def _stop_animation(self) -> None:
        if self._anim is not None:
            self.after_cancel(self._anim)
            self._anim = None

    def _wheel(self, n: int) -> None:
        self.scroll_by(n * 48)

    def scroll_by(self, pixels: float) -> None:
        """The user scrolls: following stops while the current line is out of view."""
        self._stop_animation()
        self._scroll_to(self.offset + pixels)
        self.target = self.offset
        if self.kind == lyrics_mod.SYNCED and self.entries:
            self.followed = self.current
            self._set_away(not self.line_in_view())

    def back_to_line(self) -> None:
        """The "Back to the current line" button: there at once, and following again."""
        self._stop_animation()
        self._scroll_to(self._line_target())
        self.target = self.offset
        self.followed = self.current
        self._set_away(False)

    def _line_at(self, e) -> Optional[int]:
        y = self.canvasy(e.y)
        for i, (top, h) in enumerate(zip(self.tops, self.heights)):
            if top <= y <= top + h:
                return i
        return None

    def _motion(self, e) -> None:
        self.configure(cursor="hand2" if self.kind == lyrics_mod.SYNCED and self._line_at(e) is not None else "")

    def _click(self, e) -> None:
        if self.kind != lyrics_mod.SYNCED:
            return
        i = self._line_at(e)
        if i is not None:
            self.followed = -2  # follow the line clicked
            self._set_away(False)
            self.app.seek(self.entries[i][0] / 1000.0)


class DetailsView(tk.Canvas):
    """Now Playing's Details: two columns, the labels grey on the left, scrollable.  The
    artists of the Artist row are links (`on_artist(name)`)."""

    PAD = 28

    def __init__(self, master, theme: Theme, on_artist: Optional[Callable[[str], None]] = None):
        super().__init__(master, highlightthickness=0, bd=0)
        self.theme, self.on_artist = theme, on_artist
        self.rows: List[tuple] = []
        self.artists: List[str] = []
        self.links: List[tuple] = []    # (x0, y0, x1, y1, artist) in canvas coordinates
        self.hover: Optional[int] = None
        self.offset = 0.0
        self.total = 1.0
        theme.reg(self, bg="bg")
        self.bind("<Configure>", lambda e: self.layout())
        self.bind("<Motion>", lambda e: self._hov(self._link_at(e)))
        self.bind("<Leave>", lambda e: self._hov(None))
        self.bind("<ButtonRelease-1>", self._click)
        bind_wheel(self, lambda n: self._scroll_to(self.offset + n * 40))
        theme.on_change(self.layout, self)

    def set_rows(self, rows: List[tuple], artists: Optional[List[str]] = None) -> None:
        artists = list(artists or [])
        if rows != self.rows or artists != self.artists:
            self.rows, self.artists = list(rows), artists
            self.offset = 0.0
            self.layout()

    def _link_at(self, e) -> Optional[int]:
        x, y = self.canvasx(e.x), self.canvasy(e.y)
        return next((i for i, (x0, y0, x1, y1, _) in enumerate(self.links) if x0 <= x <= x1 and y0 <= y <= y1), None)

    def _hov(self, i: Optional[int]) -> None:
        if i != self.hover:
            self.hover = i
            self.configure(cursor="hand2" if i is not None else "")
            self.delete("underline")
            if i is not None and i < len(self.links):
                x0, _, x1, y1, _ = self.links[i]
                self.create_line(x0, y1 - 1, x1, y1 - 1, fill=self.theme.c("text"), tags="underline")

    def _click(self, e) -> None:
        i = self._link_at(e)
        if i is not None and self.on_artist is not None:
            self.on_artist(self.links[i][4])

    def _artist_row(self, x: float, y: float, right: float) -> float:
        """The Artist row as links, wrapped at `right`; returns its height."""
        t = self.theme
        font, line = t.f("body"), t.f("body").metrics("linespace")
        x0, top = x, y
        for i, name in enumerate(self.artists):
            for text, link in (((", ", False),) if i else ()) + ((name, True),):
                w = font.measure(text)
                if x + w > right and x > x0 and link:
                    x, y = x0, y + line
                self.create_text(x, y, text=text, anchor="nw", font=font, fill=t.c("text"))
                if link:
                    self.links.append((x, y, x + w, y + line, name))
                x += w
        return y + line - top

    def text(self) -> str:
        return "\n".join(f"{k}: {v}" for k, v in self.rows)

    def layout(self) -> None:
        t = self.theme
        self.delete("all")
        W = self.winfo_width()
        if W <= 1:
            return
        lf, vf = t.f("body"), t.f("body")
        lw = max([lf.measure(k) for k, _ in self.rows] or [80]) + 18
        y = self.PAD / 2
        self.links, self.hover = [], None
        for k, v in self.rows:
            self.create_text(self.PAD, y, text=k, anchor="nw", font=lf, fill=t.c("text2"))
            if k == "Artist" and self.artists and v == ", ".join(self.artists):
                h = self._artist_row(self.PAD + lw, y, W - self.PAD)
            else:
                item = self.create_text(self.PAD + lw, y, text=v, anchor="nw", font=vf, fill=t.c("text"),
                                        width=max(80, W - self.PAD * 2 - lw))
                box = self.bbox(item)
                h = (box[3] - box[1]) if box else 0
            y += max(h, lf.metrics("linespace")) + 9
        self.total = y + self.PAD
        self.configure(scrollregion=(0, 0, W, max(self.total, self.winfo_height())))
        self._scroll_to(self.offset)

    def _scroll_to(self, offset: float) -> None:
        H = self.winfo_height()
        self.offset = min(max(offset, 0.0), max(0.0, self.total - H))
        self.yview_moveto(self.offset / max(self.total, H, 1))


class NowPlayingPage(Page):
    """The cover with the title and "Artist — Album" under it, as one block in the middle of
    the left half (the album name opens the album, each artist name that artist, the cover
    the Cover window);
    on the right a switch Lyrics | Up Next | Details (`app.np_tab`).  Lyrics: a label
    ("Synced lyrics" / "Lyrics (not synced)", where they come from), A− / A+ and the lyrics;
    without lyrics "Get lyrics".  With the lyrics hidden (the Lyrics button on this page,
    `app.lyrics_hidden`) only the block, in the middle of the page."""

    SIZES = (160, 200, 240, 280, 320, 360, 400, 440, 480)

    def __init__(self, app):
        super().__init__(app)
        t = self.theme
        self.art = tk.Canvas(self, highlightthickness=0, bd=0)
        t.reg(self.art, bg="bg")
        self.links: List[tuple] = []          # (what, x0, y0, x1, y1) on the art canvas
        self.hover_link: Optional[str] = None
        self.art.bind("<Configure>", lambda e: self.draw_art())
        self.art.bind("<Motion>", self._art_motion)
        self.art.bind("<Leave>", lambda e: self._art_hover(None))
        self.art.bind("<ButtonRelease-1>", self._art_click)
        self.right = frame(self, t)
        self.switch = Segmented(self.right, t, NP_TABS, app.set_np_tab, app.np_tab)
        self.switch.pack(side="top", pady=(18, 6))
        self.body = frame(self.right, t)
        self.body.pack(side="top", fill="both", expand=True)
        # Lyrics
        self.lyrics_pane = frame(self.body, t)
        head = frame(self.lyrics_pane, t)
        head.pack(side="top", fill="x", padx=(LyricsView.PAD, LyricsView.PAD - 8), pady=(6, 4))
        self.kind_l = label(head, t, "", font="body_b")
        self.kind_l.pack(side="left")
        self.source_l = label(head, t, "", font="small", fg="text2")
        self.source_l.pack(side="left", padx=(8, 0))
        self.bigger = PillButton(head, t, "A+", kind="plain", height=26, command=lambda: app.lyrics_size_step(1))
        self.bigger.pack(side="right")
        self.smaller = PillButton(head, t, "A−", kind="plain", height=26, command=lambda: app.lyrics_size_step(-1))
        self.smaller.pack(side="right", padx=(0, 6))
        self.head = head
        self.lyrics = LyricsView(self.lyrics_pane, app, on_away=self._away)
        self.lyrics.pack(side="top", fill="both", expand=True)
        self.back_btn = PillButton(self.lyrics_pane, t, "Back to the current line", command=self.lyrics.back_to_line)
        self.empty = frame(self.lyrics_pane, t)  # no lyrics: a note and "Get lyrics"
        self.empty_l = label(self.empty, t, "No lyrics for this song.", fg="text2", anchor="center")
        self.empty_l.pack(pady=(0, 10))
        self.get_btn = PillButton(self.empty, t, "Get lyrics", icon="lyrics", command=app.get_lyrics_current)
        self.get_btn.pack()
        # Up Next
        self.upnext_pane = frame(self.body, t)
        self.source = label(self.upnext_pane, t, "", "small", fg="text2")
        self.source.pack(side="top", anchor="w", padx=24, pady=(4, 6))
        self.queue_list = SongList(self.upnext_pane, t, [Column("title", "", 200, True, kind="two", key2="artist")],
                                   lambda r, k: r.display_title if k == "title" else r.display_artist,
                                   header=False, zebra=False, row_height=48, on_activate=app.jump_to,
                                   empty_text="Nothing comes after this song.")
        self.queue_list.pack(side="top", fill="both", expand=True, padx=(12, 8))
        self.queue_paths: Optional[list] = None
        # Details
        self.details = DetailsView(self.body, t, on_artist=app.open_artist)
        self.detail_extra: Dict[str, tuple] = {}  # path -> (comment, verdict), read once per song
        self.detail_busy: set = set()
        self.kind = ""
        self.label = ("", "")
        self.path: Optional[str] = None
        app.covers.listeners.append(lambda key, size: self.draw_art())
        t.on_change(self.draw_art)
        self._place()

    # ---- state -------------------------------------------------------------
    def show(self, **kw) -> None:
        self.song_changed(force=True)

    def song_changed(self, force: bool = False) -> None:
        path = self.app.current
        if path == self.path and not force:
            return
        self.path = path
        self.queue_paths = None
        if path is not None and path not in self.detail_busy:
            self.detail_extra.pop(path, None)  # its saved verdict may be new: read again
        self.load_lyrics()

    def load_lyrics(self) -> None:
        """Read the song's lyrics again and lay the page out (the song, the lyrics, the
        switch or the hidden state changed)."""
        found = lyrics_mod.read_lyrics_file(self.path) if self.path else None
        if found is None:
            self.kind, self.label = "", ("", "")
        else:
            kind, text, file = found
            self.lyrics.set_lyrics(kind, text)
            self.kind = self.lyrics.kind  # synced lyrics without a timed line show as plain
            self.label = lyrics_mod.shown_label(self.kind, self.path, file)
        self.kind_l.configure(text=self.label[0])
        self.source_l.configure(text=self.label[1])
        self.size_changed()
        self._place()
        self.draw_art()
        self.update_position(self.app.position())

    def showing_right(self) -> bool:
        return self.path is not None and not self.app.lyrics_hidden

    def showing_lyrics(self) -> bool:
        return self.showing_right() and self.app.np_tab == "lyrics" and bool(self.kind)

    def tab_changed(self) -> None:
        self.switch.set(self.app.np_tab)
        self._place()
        self.draw_art()
        self.update_position(self.app.position())

    def size_changed(self) -> None:
        """A− / A+: the lyrics fonts have a new size."""
        size = self.app.state.lyrics_size
        self.smaller.set(enabled=size > playerstate.LYRICS_SIZE_MIN)
        self.bigger.set(enabled=size < playerstate.LYRICS_SIZE_MAX)
        self.lyrics.layout()

    def _away(self, away: bool) -> None:
        if away:
            self.back_btn.place(relx=0.5, rely=1.0, anchor="s", y=-22)
            tk.Misc.tkraise(self.back_btn)  # (Canvas.lift raises canvas items)
        else:
            self.back_btn.place_forget()

    def _place(self) -> None:
        if not self.showing_right():
            self.right.place_forget()
            self.art.place(relx=0, rely=0, relwidth=1, relheight=1)
            return
        self.art.place(relx=0, rely=0, relwidth=0.5, relheight=1)
        self.right.place(relx=0.5, rely=0, relwidth=0.5, relheight=1)
        tab = self.app.np_tab
        for pane, on in ((self.lyrics_pane, tab == "lyrics"), (self.upnext_pane, tab == "upnext"),
                         (self.details, tab == "details")):
            if on:
                pane.pack(fill="both", expand=True)
            else:
                pane.pack_forget()
        if tab == "lyrics":
            if self.kind:
                self.empty.place_forget()
                if not self.lyrics.winfo_manager():
                    self.head.pack(side="top", fill="x", padx=(LyricsView.PAD, LyricsView.PAD - 8), pady=(6, 4))
                    self.lyrics.pack(side="top", fill="both", expand=True)
                self._away(self.lyrics.away)
            else:
                self.head.pack_forget()
                self.lyrics.pack_forget()
                self._away(False)
                self.empty.place(relx=0.5, rely=0.42, anchor="center")
        elif tab == "upnext":
            self.refresh_queue()
        else:
            self.refresh_details()

    # ---- Up Next and Details -----------------------------------------------------
    def refresh_queue(self) -> None:
        app = self.app
        q = app.queue
        paths = [p for p in (q.up_next(50) if q is not None else []) if p in app.by_path]
        if paths != self.queue_paths:
            self.queue_paths = paths
            self.queue_list.set_rows([app.by_path[p] for p in paths])
        self.source.configure(text=f"Playing from {app.context_name}" if q is not None and app.context_name else "")

    def refresh_details(self) -> None:
        tr = self.app.track(self.path)
        if tr is None:
            self.details.set_rows([])
            return
        album = self.app.album_of(tr)
        extra = self.detail_extra.get(tr.path)
        if extra is None:
            self._read_details(tr.path)
            extra = ("", None)
        artists = musiclib.split_artists(tr.artist, self.app.known_artists) if tr.artist else []
        self.details.set_rows(song_details(tr, len(album["tracks"]) if album else 0, extra[0], extra[1], artists),
                              artists)

    def _read_details(self, path: str) -> None:
        """The comment tag (ffprobe) and the saved verdict, in a thread; then Details again."""
        if path in self.detail_busy:
            return
        self.detail_busy.add(path)
        cfg = settings_mod.Settings.from_dict(self.app.cfg.to_dict())
        out: dict = {}

        def work():
            out["comment"] = read_comment(path)
            out["verdict"] = None
            try:
                from . import gui
                from .store import open_store, settings_key

                store = open_store(cfg)
                if store is not None:
                    try:
                        r = store.lookup(path, settings_key(cfg))
                    finally:
                        store.close()
                    if r is not None:
                        out["verdict"] = (gui.VERDICT_STYLE.get(r.verdict, (r.verdict,))[0], r.summary or "")
            except Exception:  # noqa: BLE001 - no saved results: "Not checked yet"
                pass

        def wait(th):
            if th.is_alive():
                self.after(60, lambda: wait(th))
                return
            self.detail_busy.discard(path)
            self.detail_extra[path] = (out.get("comment", ""), out.get("verdict"))
            if path == self.path and self.app.np_tab == "details":
                self.refresh_details()

        import threading

        th = threading.Thread(target=work, daemon=True, name="realhires-details")
        th.start()
        self.after(60, lambda: wait(th))

    # ---- the cover block ---------------------------------------------------------
    def draw_art(self) -> None:
        t, c = self.theme, self.art
        c.delete("all")
        self.links = []
        W, H = c.winfo_width(), c.winfo_height()
        if W <= 1:
            return
        tr = self.app.track(self.path) if self.path else None
        if tr is None:
            c.create_text(W / 2, H / 2, text="Nothing is playing", fill=t.c("text2"), font=t.f("h2"))
            return
        text_h = 80  # the title and "Artist — Album" under the cover, one block in the middle
        room = min(W - 80, H - 80 - text_h)
        size = max([s for s in self.SIZES if s <= room] or [self.SIZES[0]])
        album = self.app.album_of(tr)
        key = album["key"] if album else tr.path
        paths = [x.path for x in album["tracks"]] if album else [tr.path]
        img = self.app.covers.get(key, paths, size)
        x = (W - size) / 2
        y = (H - size - text_h) / 2
        draw_cover(c, t, img, x, y, size)
        self.links.append(("cover", x, y, x + size, y + size, None))  # the Cover window
        fit: dict = {}
        c.create_text(W / 2, y + size + 30, text=fit_text(t.f("h1"), tr.display_title, W - 60, fit),
                      fill=t.c("text"), font=t.f("h1"))
        pieces = artist_line(t.f("body"), self.app.artists_of(tr), tr.display_album if album is not None else None,
                             tr.display_album, W - 60, fit)
        font, h = t.f("body"), t.f("body").metrics("linespace")
        left, ty = W / 2 - sum(font.measure(p[0]) for p in pieces) / 2, y + size + 58
        for text, what, name in pieces:
            c.create_text(left, ty, text=text, anchor="w", fill=t.c("text2"), font=font)
            if what:
                self.links.append((what, left, ty - h / 2, left + font.measure(text), ty + h / 2, name))
            left += font.measure(text)
        self._underline()

    def _link_at(self, x: float, y: float) -> Optional[int]:
        return next((i for i, (_, x0, y0, x1, y1, _) in enumerate(self.links) if x0 <= x <= x1 and y0 <= y <= y1),
                    None)

    def _art_motion(self, e) -> None:
        self._art_hover(self._link_at(e.x, e.y))

    def _art_hover(self, i: Optional[int]) -> None:
        if i != self.hover_link:
            self.hover_link = i
            self.art.configure(cursor="hand2" if i is not None else "")
            self._underline()

    def _underline(self) -> None:
        """The artist or album name under the pointer is underlined (not the cover)."""
        self.art.delete("underline")
        i = self.hover_link
        if i is not None and i < len(self.links) and self.links[i][0] != "cover":
            _, x0, _, x1, y1, _ = self.links[i]
            self.art.create_line(x0, y1 - 1, x1, y1 - 1, fill=self.theme.c("text2"), tags="underline")

    def _art_click(self, e) -> None:
        i = self._link_at(e.x, e.y)
        tr = self.app.track(self.path) if self.path else None
        if i is None or tr is None:
            return
        what, *_, name = self.links[i]
        if what == "artist":
            self.app.open_artist(name)
        elif what == "cover":
            self.app.open_cover_window(tr.path)
        else:
            album = self.app.album_of(tr)
            if album is not None:
                self.app.open_album(album)

    # ---- every tick ----------------------------------------------------------------
    def update_position(self, seconds: float) -> None:
        if self.showing_lyrics():
            self.lyrics.update_position(seconds)
        elif self.showing_right() and self.app.np_tab == "upnext":
            self.refresh_queue()

    def lyrics_changed(self, path: str) -> None:
        if path == self.path:
            self.load_lyrics()


# ------------------------------------------------------------- settings ---
class ThemeEditor(tk.Frame):
    """Settings → Appearance: New theme…, Export…, and for the user's own themes
    Rename…, Delete…, "Dark theme" and a colour button for each of the eight
    colours, which change the app at once.  Built-in themes and those of the theme
    files in the themes folder cannot be edited here."""

    def __init__(self, master, app: "PlayerApp"):
        from . import themes

        super().__init__(master, bd=0, highlightthickness=0)
        t = app.theme
        self.app = app
        t.reg(self, bg="bg")
        theme = app.current_theme()
        own = app.is_custom_theme(theme["name"])
        self.name = theme["name"]
        self.file = "" if own else app.theme_file(theme["name"])
        label(self, t, "Theme colours", "body_b").pack(anchor="w")
        if own:
            note = f"“{theme['name']}” is your own theme: a click on a colour changes it, and the app at once."
        elif self.file:
            note = (f"“{theme['name']}” comes from the file {os.path.basename(self.file)} in the themes folder. "
                    "Change that file, or make a copy you can change here with New theme…")
        else:
            note = (f"“{theme['name']}” is a built-in theme and cannot be changed. “New theme…” makes a copy of "
                    "it that you can change.")
        self.note_l = label(self, t, note, "small", fg="text2", wraplength=SettingsPage.HELP_WIDTH, justify="left")
        self.note_l.pack(anchor="w", pady=(2, 8))
        btns = frame(self, t)
        btns.pack(anchor="w")
        acts = [("New theme…", app.new_theme), ("Export…", lambda: app.export_theme_file(self.name))]
        if own:
            acts += [("Rename…", lambda: app.rename_theme(self.name)),
                     ("Delete…", lambda: app.delete_theme(self.name))]
        self.actions = [text for text, _ in acts]
        for i, (text, cmd) in enumerate(acts):
            PillButton(btns, t, text, kind="accent" if i == 0 else "plain", command=cmd).pack(
                side="left", padx=(0 if i == 0 else 8, 0))
        if own:
            self.v_dark = tk.BooleanVar(value=bool(theme["dark"]))
            ttk.Checkbutton(self, text="Dark theme", variable=self.v_dark, style="P.TCheckbutton",
                            command=lambda: app.set_theme_dark(self.name, self.v_dark.get())).pack(anchor="w",
                                                                                                  pady=(10, 0))
        grid = frame(self, t)
        grid.pack(anchor="w", pady=(10, 0))
        self.swatches: Dict[str, tk.Canvas] = {}
        for i, key in enumerate(themes.COLOR_KEYS):
            cell = frame(grid, t)
            cell.grid(row=i // 2, column=i % 2, sticky="w", padx=(0, 28), pady=4)
            sw = tk.Canvas(cell, width=30, height=30, highlightthickness=0, bd=0, cursor="hand2")
            t.reg(sw, bg="bg")
            sw.pack(side="left")
            value = theme["colors"][key]
            round_rect(sw, 1, 1, 29, 29, 7, fill=value, outline=t.c("sep"))
            sw.bind("<Button-1>", lambda e, k=key: self._pick(k))
            box = frame(cell, t)
            box.pack(side="left", padx=(10, 0))
            label(box, t, themes.COLOR_LABELS[key], "body").pack(anchor="w")
            label(box, t, value, "small", fg="text2").pack(anchor="w")
            self.swatches[key] = sw
        self.own = own

    def _pick(self, key: str) -> None:
        from tkinter import colorchooser, messagebox

        app = self.app
        if not self.own:
            what = ("Themes from the themes folder are changed in their files." if self.file else
                    "Built-in themes cannot be changed.")
            if not messagebox.askyesno("Theme file" if self.file else "Built-in theme",
                                       f"{what} Make a copy of “{self.name}” and change that?", parent=app.root):
                return
            name = app.new_theme()
            if not name:
                return
            self.name = name
        theme = app.current_theme()
        got = colorchooser.askcolor(color=theme["colors"][key], parent=app.root,
                                    title=f"{theme['name']}: {key.capitalize()}")
        if got and got[1]:
            app.set_theme_color(self.name, key, got[1])


class SettingsPage(Page):
    """Every setting, one section per settings.GROUPS entry, plus the actions of
    the 1.x app (scan, lyrics, saved results).  Every change is saved at once."""

    HELP_WIDTH = 600

    def __init__(self, app):
        super().__init__(app)
        t = self.theme
        self.group = settings_mod.GROUPS[0]
        head = frame(self, t)
        head.pack(fill="x", padx=28, pady=(22, 8))
        label(head, t, "Settings", "title").pack(anchor="w")
        label(head, t, f"Saved in {settings_mod.default_path()}", "small", fg="text2").pack(anchor="w")
        body = frame(self, t)
        body.pack(fill="both", expand=True)
        side = frame(body, t, width=210)
        side.pack(side="left", fill="y", padx=(12, 0))
        side.pack_propagate(False)
        # under the group list: About (there is no Help menu, docs/player-spec.md 2.1)
        self.about_btn = PillButton(side, t, f"About {APP_NAME}", kind="plain", command=app.about)
        self.about_btn.pack(side="bottom", anchor="w", padx=(6, 0), pady=(8, 18))
        side.configure(width=max(210, int(self.about_btn.cget("width")) + 10))
        self.sections = SongList(side, t, [Column("name", "", 160, True)], lambda r, k: r["name"], header=False,
                                 zebra=False, on_select=self._section, row_height=34)
        self.sections.pack(side="top", fill="both", expand=True)
        self.sections.set_rows([{"name": g} for g in settings_mod.GROUPS])
        frame(body, t, bg="sep", width=1).pack(side="left", fill="y", padx=(8, 0))
        outer = frame(body, t)
        outer.pack(side="left", fill="both", expand=True)
        self.canvas = tk.Canvas(outer, highlightthickness=0, bd=0)
        t.reg(self.canvas, bg="bg")
        sb = ttk.Scrollbar(outer, orient="vertical", command=self.canvas.yview, style="P.Vertical.TScrollbar")
        self.canvas.configure(yscrollcommand=lambda a, b: (sb.set(a, b), show_scrollbar(
            sb, float(a) > 0.0 or float(b) < 1.0)))
        self.canvas.pack(side="left", fill="both", expand=True)
        sb.pack(side="right", fill="y")
        self.inner = frame(self.canvas, t)
        self.win = self.canvas.create_window(0, 0, window=self.inner, anchor="nw")
        self.inner.bind("<Configure>", lambda e: self.canvas.configure(scrollregion=self.canvas.bbox("all")))
        self.canvas.bind("<Configure>", lambda e: self.canvas.itemconfigure(self.win, width=e.width))
        bind_wheel(self.canvas, lambda n: self.canvas.yview_scroll(n, "units"))
        self.help_labels: Dict[str, tk.Label] = {}
        self.vars: Dict[str, Any] = {}
        self.sections.selected = 0
        self.build()

    def show(self, group: Optional[str] = None, **kw) -> None:
        if group in settings_mod.GROUPS:
            self.group = group
            self.sections.selected = settings_mod.GROUPS.index(group)
            self.sections.redraw()
        self.build()

    def refresh(self) -> None:
        pass

    def _section(self, i: int) -> None:
        self.group = settings_mod.GROUPS[i]
        self.build()

    # ---- building -------------------------------------------------------
    def build(self) -> None:
        t, cfg = self.theme, self.app.cfg
        for w in self.inner.winfo_children():
            w.destroy()
        t.prune()
        self.help_labels, self.vars = {}, {}
        box = frame(self.inner, t)
        box.pack(fill="both", expand=True, padx=28, pady=(6, 24))
        label(box, t, self.group, "h1").pack(anchor="w", pady=(0, 10))
        if self.group == "Detection":
            label(box, t, "Changes apply to the next scan on the Quality check page.", "body",
                  fg="text2").pack(anchor="w", pady=(0, 8))
        for info in settings_mod.Settings.field_info():
            if info["group"] == self.group:
                self._field(box, info, getattr(cfg, info["name"]))
        self._actions(box)
        self.canvas.yview_moveto(0)

    def _field(self, parent, info: dict, value) -> None:
        t = self.theme
        name, kind = info["name"], info.get("kind")
        row = frame(parent, t)
        row.pack(fill="x", anchor="w", pady=(8, 2))
        if kind == "bool":
            var = tk.BooleanVar(value=bool(value))
            ttk.Checkbutton(row, text=info["label"], variable=var, style="P.TCheckbutton",
                            command=lambda: self._set(name, var.get())).pack(anchor="w")
        else:
            label(row, t, info["label"], "body_b").pack(anchor="w", pady=(0, 4))
            if kind == "paths":
                self._paths(row, name, value)
            elif name == "output_device":
                self._device(row, value)
            elif name == "font_family":
                self._font(row, value)
            elif name == "theme":
                self._theme(row, value)
            elif kind == "choice":
                var = tk.StringVar(value=value)
                width = max(18, max(len(c) for c in info["choices"]) + 2)
                cb = ttk.Combobox(row, textvariable=var, values=info["choices"], state="readonly", width=width,
                                  style="P.TCombobox", font=t.f("body"))
                cb.pack(anchor="w")
                cb.bind("<<ComboboxSelected>>", lambda e: self._set(name, var.get()))
            elif kind == "multi":
                from .gui import VERDICT_STYLE

                sub = frame(row, t)
                sub.pack(anchor="w")
                bools = {}
                for i, choice in enumerate(info["choices"]):
                    b = tk.BooleanVar(value=choice in value)
                    bools[choice] = b
                    ttk.Checkbutton(sub, text=VERDICT_STYLE.get(choice, (choice,))[0], variable=b,
                                    style="P.TCheckbutton",
                                    command=lambda: self._set(name, [c for c, v in bools.items() if v.get()])
                                    ).grid(row=i // 4, column=i % 4, sticky="w", padx=(0, 14))
            elif kind == "str":
                var = tk.StringVar(value=value)
                e = ttk.Entry(row, textvariable=var, width=60, style="P.TEntry", font=t.f("body"))
                e.pack(anchor="w")
                for ev in ("<FocusOut>", "<Return>"):
                    e.bind(ev, lambda _e: self._set(name, var.get()))
            else:
                var = tk.StringVar(value=f"{value:g}" if isinstance(value, float) else str(value))
                inc = 1 if kind == "int" or (info.get("max") or 0) > 50 else 0.1
                sp = ttk.Spinbox(row, textvariable=var, from_=info.get("min") or 0, to=info.get("max") or 1e9,
                                 increment=inc, width=10, style="P.TSpinbox", font=t.f("body"),
                                 command=lambda: self._set_number(name, var, kind))
                sp.pack(anchor="w")
                for ev in ("<FocusOut>", "<Return>"):
                    sp.bind(ev, lambda _e: self._set_number(name, var, kind))
        help_l = label(row, t, info["help"], "small", fg="text2", wraplength=self.HELP_WIDTH, justify="left")
        help_l.pack(anchor="w", pady=(3, 0))
        self.help_labels[name] = help_l
        if name == "theme":
            files = frame(row, t)
            files.pack(anchor="w", fill="x", pady=(10, 0))
            label(files, t, "Theme files (.json) put into the themes folder appear in this menu.", "small",
                  fg="text2").pack(side="left")
            PillButton(files, t, "Import theme…", kind="plain", command=self.app.import_theme_file).pack(
                side="left", padx=(12, 0))
            PillButton(files, t, "Open themes folder", kind="plain", command=self.app.open_themes_folder).pack(
                side="left", padx=(8, 0))
            self.theme_editor = ThemeEditor(row, self.app)
            self.theme_editor.pack(anchor="w", fill="x", pady=(12, 0))

    def _theme(self, parent, value: str) -> None:
        t = self.theme
        names = self.app.theme_names()
        var = tk.StringVar(value=value if value in names else names[0])
        cb = ttk.Combobox(parent, textvariable=var, values=names, state="readonly",
                          width=max(24, max(len(n) for n in names) + 2), style="P.TCombobox", font=t.f("body"),
                          postcommand=self.theme_values)
        cb.pack(anchor="w")
        cb.bind("<<ComboboxSelected>>", lambda e: self.app.select_theme(var.get()))
        self.theme_box = cb

    def theme_values(self) -> List[str]:
        """The theme menu again, when it opens: theme files may have been put into the
        themes folder or taken out of it."""
        names = self.app.theme_names()
        self.theme_box.configure(values=names, width=max(24, max(len(n) for n in names) + 2))
        return names

    def _paths(self, parent, name: str, value: List[str]) -> None:
        t = self.theme
        lb = tk.Listbox(parent, height=max(3, min(len(value), 8)), width=70, activestyle="none",
                        relief="flat", bd=0, highlightthickness=1, font=t.f("body"), exportselection=False)
        t.reg(lb, bg="lcd", fg="text", selectbackground="accent", selectforeground="on_accent",
              highlightbackground="sep", highlightcolor="sep")
        for p in value:
            lb.insert("end", p)
        if not value:
            lb.insert("end", "No music folder yet")
            lb.itemconfigure(0, foreground=t.c("text2"))
        lb.pack(anchor="w")
        btns = frame(parent, t)
        btns.pack(anchor="w", pady=(6, 0))
        PillButton(btns, t, "Add folder…", kind="plain", command=self.app.choose_music_folder).pack(side="left")

        def remove():
            sel = [lb.get(i) for i in lb.curselection()]
            keep = [p for p in self.app.cfg.library_folders if p not in sel]
            if sel and keep != self.app.cfg.library_folders:
                self._set(name, keep)
                self.build()

        PillButton(btns, t, "Remove", kind="plain", command=remove).pack(side="left", padx=(8, 0))

    def _device(self, parent, value: str) -> None:
        t = self.theme
        names = ["System default"] + [d.name for d in self.app.output_devices()]
        if value and value not in names:
            names.append(value)
        var = tk.StringVar(value=value or "System default")
        cb = ttk.Combobox(parent, textvariable=var, values=names, state="readonly",
                          width=max(30, max(len(n) for n in names) + 2), style="P.TCombobox", font=t.f("body"))
        cb.pack(anchor="w")
        cb.bind("<<ComboboxSelected>>",
                lambda e: self._set("output_device", "" if var.get() == "System default" else var.get()))

    def _font(self, parent, value: str) -> None:
        t = self.theme
        names = [DEFAULT_FONT_LABEL] + font_families(self)
        var = tk.StringVar(value=value or DEFAULT_FONT_LABEL)
        cb = ttk.Combobox(parent, textvariable=var, values=names, state="readonly", width=36, style="P.TCombobox",
                          font=t.f("body"))
        cb.pack(anchor="w")
        cb.bind("<<ComboboxSelected>>",
                lambda e: self._set("font_family", "" if var.get() == DEFAULT_FONT_LABEL else var.get()))

    def _actions(self, parent) -> None:
        t, app = self.theme, self.app
        acts = []
        if self.group == "Library":
            acts = [("Update the library now", app.refresh_library),
                    ("Scan library", lambda: app.check_library(switch=True)),
                    ("Download lyrics for the whole library…", app.lyrics_for_library),
                    ("Open the quality checker", lambda: app.show_page("quality")),
                    ("Clear the play history…", app.clear_history)]
        elif self.group == "Playback":
            acts = [("Equalizer…", app.open_eq)]
        elif self.group == "Saved results":
            acts = [("Delete all saved results…", app.qc.forget_saved)]
            st = app.qc.store.stats() if app.qc.store is not None else None
            from .store import default_path as store_path

            where = (f"{st['files']:,} file(s), {st['disk_bytes'] / 1e6:.1f} MB, in {st['path']}" if st
                     else f"Saving is turned off. Results are kept in {store_path()}")
            label(parent, t, where, "small", fg="text2", wraplength=self.HELP_WIDTH, justify="left").pack(
                anchor="w", pady=(14, 0))
        elif self.group in ("Detection", "Scanning", "Display", "Fixing"):
            acts = [("Open the quality checker", lambda: app.show_page("quality"))]
        if acts:
            row = frame(parent, t)
            row.pack(anchor="w", pady=(18, 0))
            for i, (text, cmd) in enumerate(acts):  # two per row, so they never run off the page
                PillButton(row, t, text, kind="accent" if i == 0 else "plain", command=cmd).grid(
                    row=i // 2, column=i % 2, sticky="w", padx=(0, 10), pady=(0, 8))

    # ---- saving ---------------------------------------------------------
    def _set_number(self, name: str, var: tk.StringVar, kind: Optional[str]) -> None:
        raw = var.get().strip()
        try:
            value = int(raw) if kind == "int" else float(raw)
        except ValueError:
            self._bad(name)
            return
        if value != getattr(self.app.cfg, name):
            self._set(name, value)

    def _bad(self, name: str) -> None:
        info = next(i for i in settings_mod.Settings.field_info() if i["name"] == name)
        lo, hi = info.get("min"), info.get("max")
        rng = f" from {lo:g} to {hi:g}" if lo is not None and hi is not None else ""
        lab = self.help_labels.get(name)
        if lab is not None:
            lab.configure(text=f"Please enter a number{rng}. {info['help']}", fg=self.theme.c("error"))

    def _set(self, name: str, value) -> None:
        cfg = self.app.cfg
        data = cfg.to_dict()
        data[name] = value
        new = settings_mod.Settings.from_dict(data)
        got = getattr(new, name)
        same = got == value or (isinstance(got, (int, float)) and isinstance(value, (int, float))
                                and float(got) == float(value))
        if not same:
            self._bad(name)
            return
        lab = self.help_labels.get(name)
        if lab is not None:
            info = next(i for i in settings_mod.Settings.field_info() if i["name"] == name)
            lab.configure(text=info["help"], fg=self.theme.c("text2"))
        self.app.apply_settings(new)


__all__ = ["Page", "AlbumsPage", "AlbumPage", "GroupPage", "SongsPage", "SearchPage",
           "NowPlayingPage", "SettingsPage", "LyricsView", "track_value", "track_sort", "album_order"]
