"""Search Cover Online → MusicBrainz… in the Tk player (2.5, docs/player-spec.md, "2.5", item 9).

`CoverSearchWindow` belongs to the Cover window of a song's album (it is opened with it
and closed with it): it looks the album up on MusicBrainz (`coverart.find_releases()`,
through the existing client and its one request a second) and shows the front covers of
the releases found, from the Cover Art Archive, as a grid of thumbnails with each
release's title, artist, year, country and format.  A click on one downloads the cover
as it was uploaded and shows it large with its size in pixels; Use this cover hands it
to the Cover window, which sets it exactly as Change cover… does (for the songs chosen
there: the choice is shown here too).

One thread does everything, one request at a time: the search, then the thumbnails in
order, a chosen cover first.  Results reach Tk through `self.q`.  Only the album's
artist and title are sent, never a path.
"""

from __future__ import annotations

import base64
import queue
import threading
import tkinter as tk
from tkinter import ttk
from typing import TYPE_CHECKING, Dict, List, Optional

from . import coverart, covers, tagedit
from . import musicbrainz as mb
from .i18n import tr, tr_text
from .player_pages import frame, label
from .player_theme import PillButton, fit_text
from .player_widgets import bind_wheel
from .player_windows import _window, close_window

if TYPE_CHECKING:  # pragma: no cover
    from .player_gui import PlayerApp


class CoverSearchWindow:
    """See the module."""

    THUMB = 148      # a thumbnail in the grid, in pixels
    GAP = 14
    CAPTION = 54     # three lines under it: title, artist, year · country · format
    LARGE = 400      # the chosen cover, at most

    def __init__(self, app: "PlayerApp", cover_window):
        self.app, t = app, app.theme
        self.cw = cover_window
        self.path = cover_window.path
        self.q: "queue.Queue" = queue.Queue()
        self.stop = threading.Event()
        self.mb = mb.MusicBrainzClient(app.cfg.musicbrainz_server, stop=self.stop)
        self.art = coverart.CoverArtClient(stop=self.stop)
        self.gen = 0                              # of the search whose results are shown
        self.searching = False
        self.releases: List[mb.Release] = []      # found on MusicBrainz
        self.asked = 0                            # of them, how many were asked of the archive
        self.tiles: List[dict] = []               # those with a cover: {"release", "photo"}
        self.boxes: List[tuple] = []              # where the tiles are drawn: (index, x0, y0, x1, y1)
        self.hover: Optional[int] = None
        self.chosen: Optional[mb.Release] = None  # the release whose cover is shown large
        self.picture: Optional[bytes] = None      # that cover as it was uploaded
        self.photo: Optional[tk.PhotoImage] = None
        self.loading = False
        self.full: Dict[str, tuple] = {}          # release id -> (picture, its PNG to show, details): never twice
        self.error_text = ""
        self.done = False                         # every thumbnail was asked for
        self._cv = threading.Condition()
        self._jobs: List[tuple] = []              # ("search", gen, album, artist) / ("thumb", gen, release)
        self._full: Optional[tuple] = None        # (gen, release): the chosen cover, before the thumbnails
        track = cover_window.track
        album, artist = coverart.search_terms(track.album_artist, track.artist, track.album,
                                              track.title or track.display_title)
        self.win = _window(app, tr("Search Cover on MusicBrainz"))
        self.win.transient(cover_window.win)
        self.win.protocol("WM_DELETE_WINDOW", self.close)
        self.win.bind("<<Close>>", lambda e: self.close())
        self.win.resizable(True, True)
        box = frame(self.win, t)
        box.pack(fill="both", expand=True, padx=22, pady=16)
        row = frame(box, t)
        row.pack(fill="x")
        label(row, t, tr("Album"), "body", fg="text2").pack(side="left")
        self.v_album = tk.StringVar(value=album)
        self.e_album = ttk.Entry(row, textvariable=self.v_album, width=26, style="P.TEntry", font=t.f("body"))
        self.e_album.pack(side="left", padx=(8, 14), fill="x", expand=True)
        label(row, t, tr("Artist"), "body", fg="text2").pack(side="left")
        self.v_artist = tk.StringVar(value=artist)
        self.e_artist = ttk.Entry(row, textvariable=self.v_artist, width=20, style="P.TEntry", font=t.f("body"))
        self.e_artist.pack(side="left", padx=(8, 12), fill="x", expand=True)
        for e in (self.e_album, self.e_artist):
            e.bind("<Return>", lambda ev: self.search())
            e.bind("<KP_Enter>", lambda ev: self.search())
        self.search_b = PillButton(row, t, tr("Search"), command=self.search)
        self.search_b.pack(side="left")
        self.status = label(box, t, "", "small", fg="text2", wraplength=640, justify="left")
        self.status.pack(anchor="w", pady=(8, 6))
        bottom = frame(box, t)  # packed before the middle, so a small window shrinks the pictures, not this
        bottom.pack(fill="x", side="bottom")
        self.credit = label(bottom, t, tr_text(coverart.CREDIT), "small", fg="text2", wraplength=640, justify="left")
        self.credit.pack(anchor="w", side="bottom", pady=(8, 0))
        self.btns = frame(bottom, t)
        self.btns.pack(fill="x", side="bottom", pady=(10, 0))
        self.close_b = PillButton(self.btns, t, tr("Close"), kind="plain", command=self.close)
        self.close_b.pack(side="right")
        self.use_b = PillButton(self.btns, t, tr("Use this cover"), command=self.use)
        self.back_b = PillButton(self.btns, t, tr("Back to the covers"), kind="plain", command=self.back)
        # the choice of the Cover window, shown with the chosen cover (the same variables)
        self.opts = frame(bottom, t)
        n = len(cover_window.album_paths)
        if n > 1:
            ttk.Radiobutton(self.opts, text=tr("All {n} songs of this album", n=f"{n:,}"), value="album",
                            variable=cover_window.scope, style="P.TRadiobutton").pack(anchor="w")
            ttk.Radiobutton(self.opts, text=tr("This song only"), value="song", variable=cover_window.scope,
                            style="P.TRadiobutton").pack(anchor="w")
        ttk.Checkbutton(self.opts, text=tr("Also save it as cover.jpg in the album's folder"),
                        variable=cover_window.also_folder, style="P.TCheckbutton").pack(anchor="w", pady=(4, 0))
        self.details = label(bottom, t, "", "body")
        self.caption = label(bottom, t, "", "body", fg="text2", wraplength=640, justify="left")
        self.canvas = tk.Canvas(box, width=4 * (self.THUMB + self.GAP), height=2 * (self.THUMB + self.CAPTION + self.GAP),
                                highlightthickness=0, bd=0)
        t.reg(self.canvas, bg="bg")
        self.canvas.pack(fill="both", expand=True)
        self.canvas.bind("<Configure>", lambda e: self.draw())
        self.canvas.bind("<Motion>", self._motion)
        self.canvas.bind("<Leave>", lambda e: self._hover(None))
        self.canvas.bind("<ButtonRelease-1>", self._click)
        bind_wheel(self.canvas, self._wheel)
        self.offset = 0
        t.on_change(self.draw, self.win)
        self.win.minsize(560, 420)
        threading.Thread(target=self._work, daemon=True, name="realhires-cover-search").start()
        self.win.after(80, self._poll)
        if album:
            self.search()
        else:
            self._status(tr("Type the album's name and press Search."))

    # ---- the thread ----------------------------------------------------------
    def _next(self) -> Optional[tuple]:
        with self._cv:
            while not self.stop.is_set():
                if self._full is not None:
                    job, self._full = ("full",) + self._full, None
                    return job
                if self._jobs:
                    return self._jobs.pop(0)
                self._cv.wait(0.5)
        return None

    def _work(self) -> None:
        while True:
            job = self._next()
            if job is None:
                return
            kind, gen = job[0], job[1]
            if gen != self.gen:
                continue  # of an earlier search
            try:
                if kind == "search":
                    found = coverart.find_releases(self.mb, job[2], job[3])
                    with self._cv:
                        if gen == self.gen:
                            self._jobs.extend(("thumb", gen, r) for r in found)
                    self.q.put(("found", gen, found))
                elif kind == "thumb":
                    r = job[2]
                    data = self.art.front(r.id, True)
                    png = covers.fit_png(data, self.THUMB, enlarge=True) if data else None
                    self.q.put(("thumb", gen, r, png))
                else:
                    r = job[2]
                    data = self.art.front(r.id, False)
                    if data is None:
                        raise coverart.CoverArtError("The Cover Art Archive has no cover for this release.")
                    info = tagedit.check_picture(data)
                    text = covers.details_text(info.width, info.height, covers.image_format(data), len(data))
                    self.q.put(("full", gen, r, data, covers.fit_png(data, self.LARGE), text))
            except (mb.MusicBrainzStopped, coverart.CoverArtStopped):
                return
            except coverart.CoverArtBusy as exc:  # nothing more is asked of the archive now
                with self._cv:
                    self._jobs = [j for j in self._jobs if j[0] != "thumb"]
                self.q.put(("error", gen, kind, str(exc), True))
            except (mb.MusicBrainzError, coverart.CoverArtError, tagedit.TagError) as exc:
                self.q.put(("error", gen, kind, str(exc), False))
            except Exception as exc:  # noqa: BLE001 - a picture that cannot be read must not end the thread
                self.q.put(("error", gen, kind, str(exc), False))

    def _poll(self) -> None:
        try:
            if not self.win.winfo_exists():
                return
        except tk.TclError:
            return
        try:
            while True:
                kind, gen, *rest = self.q.get_nowait()
                if gen != self.gen:
                    continue
                if kind == "found":
                    self.releases, self.searching = rest[0], False
                    self.done = not self.releases
                elif kind == "thumb":
                    r, png = rest
                    self.asked += 1
                    if png:
                        try:
                            photo = tk.PhotoImage(master=self.canvas, data=base64.b64encode(png).decode("ascii"),
                                                  format="png")
                            self.tiles.append({"release": r, "photo": photo})
                        except tk.TclError:
                            pass
                    self.done = self.asked >= len(self.releases)
                elif kind == "full":
                    r, data, png, text = rest
                    self.full[r.id] = (data, png, text)
                    if self.chosen is not None and self.chosen.id == r.id:
                        self._show_full(r.id)
                elif kind == "error":
                    what, text, busy = rest
                    if what == "search":
                        self.searching, self.done = False, True
                    elif what == "thumb":
                        self.asked += 1
                        if busy:
                            self.asked = len(self.releases)
                        self.done = self.asked >= len(self.releases)
                    else:
                        self.loading = False
                    if what != "thumb" or busy:  # one thumbnail that fails is just not shown
                        self.error_text = tr_text(text)
                self._update()
        except queue.Empty:
            pass
        self.win.after(80, self._poll)

    def _show_full(self, rid: str) -> None:
        data, png, text = self.full[rid]
        self.loading, self.picture = False, data
        try:
            self.photo = tk.PhotoImage(master=self.canvas, format="png",
                                       data=base64.b64encode(png or b"").decode("ascii"))
        except tk.TclError:
            self.photo = None
        self.details.configure(text=tr_text(text))

    # ---- the window ------------------------------------------------------------
    def _status(self, text: str) -> None:
        self.status.configure(text=text)

    def status_text(self) -> str:
        if self.error_text:
            return self.error_text
        album = self.v_shown
        if self.searching:
            return tr("Looking for “{album}” on MusicBrainz…", album=album)
        n, k = len(self.releases), len(self.tiles)
        if not n:
            return tr("MusicBrainz knows no release called “{album}”. Change the words and search again.", album=album)
        if not self.done:
            return tr("Getting the covers from the Cover Art Archive ({i} of {n})…", i=min(self.asked + 1, n), n=n)
        if not k:
            return (tr("MusicBrainz knows 1 release, but the Cover Art Archive has no cover for it.") if n == 1 else
                    tr("MusicBrainz knows {n} releases, but the Cover Art Archive has no cover for them.", n=n))
        return tr("1 cover found. Click it to see it large.") if k == 1 else \
            tr("{n} covers found. Click one to see it large.", n=k)

    def _update(self) -> None:
        self._status(self.status_text())
        self.search_b.set(enabled=not self.searching)
        large = self.chosen is not None
        for w in (self.use_b, self.back_b, self.opts, self.details, self.caption):
            w.pack_forget()
        if large:
            self.back_b.pack(side="left")
            self.use_b.pack(side="right", padx=(0, 8))
            self.opts.pack(fill="x", side="bottom", pady=(8, 0))
            self.caption.pack(anchor="w", side="bottom")
            self.details.pack(anchor="w", side="bottom")
            self.use_b.set(enabled=self.can_use())
        self.draw()

    def can_use(self) -> bool:
        return self.picture is not None and not self.loading and not self.cw.busy and not self.cw.reading

    def columns(self) -> int:
        return max(1, (max(self.canvas.winfo_width(), 1) + self.GAP) // (self.THUMB + self.GAP))

    def draw(self) -> None:
        c, t = self.canvas, self.app.theme
        try:
            c.delete("all")
            W, H = c.winfo_width(), c.winfo_height()
        except tk.TclError:
            return
        self.boxes = []
        if self.chosen is not None:
            if self.photo is not None:
                photo = self.photo
                room = max(60, min(W, H) - 4)
                if photo.width() > room or photo.height() > room:  # a small window: shrink by whole steps
                    f = max(-(-photo.width() // room), -(-photo.height() // room))
                    photo = self._small = photo.subsample(f, f)
                c.create_image(W / 2, H / 2, image=photo, anchor="center", tags=("large",))
            else:
                text = tr("Getting the cover…") if self.loading else tr("The picture cannot be shown.")
                c.create_text(W / 2, H / 2, text=text, fill=t.c("text2"), font=t.f("body"))
            return
        cols = self.columns()
        step_x, step_y = self.THUMB + self.GAP, self.THUMB + self.CAPTION + self.GAP
        rows = -(-len(self.tiles) // cols)
        self.offset = min(max(self.offset, 0), max(0, rows * step_y - H))
        left = max(0, (W - (cols * step_x - self.GAP)) // 2)
        bold, small = t.f("body_b"), t.f("small")
        fit: dict = {}
        for i, tile in enumerate(self.tiles):
            x = left + (i % cols) * step_x
            y = (i // cols) * step_y - self.offset
            if y + step_y < 0 or y > H:
                continue
            r = tile["release"]
            if i == self.hover:
                c.create_rectangle(x - 5, y - 5, x + self.THUMB + 5, y + self.THUMB + self.CAPTION, fill=t.c("hover"),
                                   outline="")
            c.create_image(x, y, image=tile["photo"], anchor="nw", tags=("thumb",))
            c.create_text(x, y + self.THUMB + 12, text=fit_text(bold, r.title, self.THUMB, fit), anchor="w",
                          fill=t.c("text"), font=bold, tags=("title",))
            c.create_text(x, y + self.THUMB + 28, text=fit_text(small, r.artist, self.THUMB, fit), anchor="w",
                          fill=t.c("text2"), font=small)
            c.create_text(x, y + self.THUMB + 43, text=fit_text(small, coverart.release_facts(r), self.THUMB, fit),
                          anchor="w", fill=t.c("text2"), font=small, tags=("facts",))
            self.boxes.append((i, x, y, x + self.THUMB, y + self.THUMB + self.CAPTION))

    def _at(self, x: float, y: float) -> Optional[int]:
        return next((i for i, x0, y0, x1, y1 in self.boxes if x0 <= x <= x1 and y0 <= y <= y1), None)

    def _hover(self, i: Optional[int]) -> None:
        if i != self.hover:
            self.hover = i
            self.canvas.configure(cursor="hand2" if i is not None else "")
            self.draw()

    def _motion(self, e) -> None:
        self._hover(self._at(e.x, e.y))

    def _click(self, e) -> None:
        i = self._at(e.x, e.y)
        if i is not None:
            self.choose(i)

    def _wheel(self, n: int) -> None:
        if self.chosen is None:
            self.offset += n * 60
            self.draw()

    # ---- what the user does ------------------------------------------------------
    @property
    def v_shown(self) -> str:
        return getattr(self, "_searched", "") or self.v_album.get().strip()

    def search(self) -> None:
        """Search: the releases called so on MusicBrainz, then their covers."""
        album, artist = self.v_album.get().strip(), self.v_artist.get().strip()
        if not album or self.searching:
            if not album:
                self._status(tr("Type the album's name and press Search."))
            return
        self._searched = album
        self.gen += 1
        self.releases, self.tiles, self.asked, self.done = [], [], 0, False
        self.chosen, self.picture, self.photo, self.loading = None, None, None, False
        self.error_text, self.searching, self.offset, self.hover = "", True, 0, None
        with self._cv:
            self._jobs = [("search", self.gen, album, artist)]
            self._full = None
            self._cv.notify()
        self._update()

    def choose(self, i: int) -> None:
        """A click on a cover: it is downloaded as it was uploaded and shown large."""
        if not 0 <= i < len(self.tiles):
            return
        r = self.tiles[i]["release"]
        self.chosen, self.picture, self.photo, self.loading, self.error_text = r, None, None, True, ""
        self.hover = None
        self.canvas.configure(cursor="")
        self.details.configure(text="")
        facts = coverart.release_facts(r)
        head = tr("{title} — {artist}", title=r.title, artist=r.artist) if r.artist else r.title
        self.caption.configure(text=f"{head}  ·  {facts}" if facts else head)
        if r.id in self.full:  # downloaded before
            self._show_full(r.id)
        else:
            with self._cv:
                self._full = (self.gen, r)
                self._cv.notify()
        self._update()

    def back(self) -> None:
        """Back to the covers."""
        self.chosen, self.picture, self.photo, self.loading, self.error_text = None, None, None, False, ""
        with self._cv:
            self._full = None
        self._update()

    def use(self) -> bool:
        """Use this cover: the Cover window sets it, as Change cover… does; this window closes."""
        if not self.can_use():
            return False
        data, r = self.picture, self.chosen
        cw = self.cw
        self.close()
        return cw.change_to_data(data, tr("The cover of “{album}” from the Cover Art Archive", album=r.title))

    def close(self) -> None:
        self.stop.set()
        with self._cv:
            self._cv.notify_all()
        if getattr(self.app, "cover_search", None) is self:
            self.app.cover_search = None
        close_window(self.win)


__all__ = ["CoverSearchWindow"]
