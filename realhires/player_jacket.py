"""Now Playing's cover as a record sleeve (2.5, docs/player-spec.md, "2.5", items 2 and 9).

`Jacket` draws the square of Now Playing's cover on the page's canvas: the cover, or its
back (the album's songs over the cover mirrored and blurred, `covers.back_picture()`,
never larger than the square), and the turn between them: a swipe, the dots, a sideways
scroll or the arrow keys turn the jacket about its vertical axis (`jacket.turn_frame()`),
drawn by scaling the two prepared pictures column by column with NumPy
(`turn_picture()`), one PhotoImage a frame.  The pictures of a cover are made once, in a
thread (FFmpeg decodes the cached PNG), and kept for the session.

`jacket_menu()` is the right-click menu of a cover (Cover…, Change Cover…, Search Cover
Online ▸, Go to Album) and `add_search_menu()` the Search Cover Online ▸ entry that song
and album menus share.
"""

from __future__ import annotations

import threading
import time
import tkinter as tk
import tkinter.font as tkfont
from typing import TYPE_CHECKING, Dict, List, Optional, Tuple

import numpy as np

from . import covers, jacket
from .i18n import tr, tr_name
from .imaging import to_ppm
from .player_theme import draw_icon, fit_text
from .player_widgets import draw_cover, fmt_time

if TYPE_CHECKING:  # pragma: no cover
    from .player_gui import PlayerApp

DRAG_START_PX = 5   # a press that moves further is a drag, not a click
FRAME_MS = 8        # the pause between two frames of the turn (drawing one takes about as long)
WHITE = (255, 255, 255)


def rgb_of(color: str) -> Tuple[int, int, int]:
    """"#rrggbb" as (r, g, b)."""
    c = color.lstrip("#")
    return (int(c[0:2], 16), int(c[2:4], 16), int(c[4:6], 16))


def hex_of(rgb) -> str:
    return "#%02x%02x%02x" % tuple(int(v) for v in rgb)


def corner_radius(size: int) -> int:
    """The radius of a cover's rounded corners (as `rounded_photo()` cuts them)."""
    return max(3, int(round(size * (0.06 if size <= 64 else 0.045))))


def corner_mask(size: int) -> np.ndarray:
    """(size, size) bool: the pixels outside the rounded corners."""
    r = corner_radius(size)
    k = np.arange(size)
    d = np.minimum(k, size - 1 - k)                      # distance to the nearer edge
    off = np.where(d < r, r - d - 0.5, 0.0)              # how far inside the corner's box
    return (off[:, None] ** 2 + off[None, :] ** 2 > r * r) & (d[:, None] < r) & (d[None, :] < r)


def on_background(face: np.ndarray, bg: Tuple[int, int, int]) -> np.ndarray:
    """The picture with its corners rounded off into the background colour."""
    out = np.array(face, dtype=np.uint8, copy=True)
    out[corner_mask(out.shape[0])] = bg
    return out


def pack(face: np.ndarray) -> np.ndarray:
    """A (size, size, 3) picture as (size, size) four-byte pixels: one lookup a pixel is
    several times quicker than three bytes each, which a frame of the turn needs."""
    size = face.shape[0]
    packed = np.zeros((size, face.shape[1], 4), dtype=np.uint8)
    packed[:, :, :3] = face
    return packed.view(np.uint32).reshape(size, face.shape[1])


def turn_picture(face: np.ndarray, frame: jacket.TurnFrame, bg: Tuple[int, int, int]) -> np.ndarray:
    """One frame of the turn: `face` (size x size x 3 on the background colour, or packed
    with `pack()`) drawn `frame.width` wide about the middle, its far edge shorter (the
    perspective), darkened, on the background colour.  Never larger than the square."""
    if face.ndim == 3:
        face = pack(face)
    size = face.shape[0]
    bg32 = pack(np.array([[bg]], dtype=np.uint8))[0, 0]
    out = np.full((size, size), bg32, dtype=np.uint32)
    w = min(size, max(2, int(round(size * frame.width))))  # edge-on: the sleeve's own thickness
    x0 = (size - w) // 2
    t = (np.arange(w, dtype=np.float32) + 0.5) / w         # 0 at the left edge, 1 at the right
    u = np.minimum((t * size).astype(np.int32), size - 1)  # the picture's column for each column drawn
    away = t if frame.far_right else 1.0 - t               # 0 at the near edge, 1 at the far one
    height = size * (1.0 - (1.0 - frame.far) * away)
    top = (size - height) / 2.0
    bottom = top + height
    rows = np.arange(size, dtype=np.float32)[:, None]
    v = np.floor((rows + 0.5 - top[None, :]) * (size / height)[None, :]).astype(np.int32)
    inside = (v >= 0) & (v < size)
    np.clip(v, 0, size - 1, out=v)
    v *= size
    v += u[None, :]
    pix = np.take(face.reshape(-1), v)
    keep = np.uint32(int(round((1.0 - frame.dark) * 256)))
    if keep < 256:  # darkened: the four bytes of a pixel two at a time
        low = (((pix & 0x00FF00FF) * keep) >> 8) & 0x00FF00FF
        high = ((((pix >> 8) & 0x00FF00FF) * keep) >> 8) & 0x00FF00FF
        pix = low | (high << 8)
    part = out[:, x0:x0 + w]
    np.copyto(part, pix, where=inside)
    rgb = out.view(np.uint8).reshape(size, size, 4)
    # the slanted top and bottom edges, smoothed by how much of their pixel the face covers
    back = np.array(bg, dtype=np.float32)
    cols = np.arange(w)
    for edge, share in ((np.floor(top), 1.0 - (top - np.floor(top))), (np.floor(bottom), bottom - np.floor(bottom))):
        r = np.clip(edge, 0, size - 1).astype(np.intp)
        share = np.clip(share, 0.0, 1.0)[:, None].astype(np.float32)
        mine = pix[r, cols].view(np.uint8).reshape(w, 4)[:, :3]
        rgb[r, cols + x0, :3] = (mine * share + back * (1.0 - share) + 0.5).astype(np.uint8)
    return rgb[:, :, :3]


class Faces:
    """The two pictures of one cover at one size: the cover and its back (scrimmed)."""

    def __init__(self, front: np.ndarray, back: np.ndarray, source=None):
        self.front, self.back = front, back
        self.source = source  # the cover's picture in the cover cache that they were made for
        self.average = tuple(int(v) for v in back.reshape(-1, 3).mean(axis=0))
        self.cut: Dict[tuple, tuple] = {}  # background colour -> the two with rounded corners, and packed

    def on(self, bg: Tuple[int, int, int]) -> tuple:
        """(front, back, front packed, back packed), their corners rounded off into `bg`."""
        if bg not in self.cut:
            front, back = on_background(self.front, bg), on_background(self.back, bg)
            self.cut = {bg: (front, back, pack(front), pack(back))}
        return self.cut[bg]


def flat_faces(size: int, color: Tuple[int, int, int]) -> Faces:
    """An album without a cover: the placeholder's colour, and a darker back."""
    front = np.empty((size, size, 3), dtype=np.uint8)
    front[:] = color
    return Faces(front, covers.darken(front, covers.BACK_SCRIM))


class Jacket:
    """The square of Now Playing's cover: front, back and the turn (see the module)."""

    MAX_KEPT = 12  # covers whose pictures are kept (each up to 480 x 480, twice)

    def __init__(self, page):
        self.page, self.app, self.c = page, page.app, page.art
        self.theme = self.app.theme
        self.box: Optional[tuple] = None          # (x, y, size) of the square, on the canvas
        self.album: Optional[dict] = None
        self.key = ""                             # the album (or the song) whose cover it is
        self.paths: List[str] = []
        self.img: Optional[tk.PhotoImage] = None  # the cover as the player's cover cache has it
        self.turn: Optional[dict] = None          # the turn going on: from, to, dir, p, anim, drag
        self.fade_t0: Optional[float] = None      # since when the back's text is fading in
        self.first = 0                            # the first of the album's songs shown (scrolled)
        self.first_key: Optional[str] = None      # the album they are of
        self.hover: Optional[int] = None          # the song under the pointer
        self.over = False                         # the pointer is over the square (for the arrow keys)
        self.press: Optional[tuple] = None        # where the button went down
        self.dragged = False
        self.faces: Dict[tuple, Faces] = {}       # (key, size) -> the pictures
        self.pending: set = set()
        self.done: List[tuple] = []               # what the threads made: (key, size, front, back)
        self.frames = 0                           # frames of turns drawn (the tests count them)
        self.last_frame: Optional[jacket.TurnFrame] = None
        self._images: List[tk.PhotoImage] = []    # what the canvas shows now (kept alive)
        self._back_img: Optional[tuple] = None    # ((key, size, bg), PhotoImage)
        self._fonts: Dict[tuple, tkfont.Font] = {}
        self._job: Optional[str] = None
        self._poll_job: Optional[str] = None

    # ---- the pictures ----------------------------------------------------------
    def _faces(self) -> Optional[Faces]:
        """The pictures of the cover shown, if they are ready; else they are asked for.
        They belong to the cover's picture in the player's cover cache (`self.img`): a
        cover that was changed gets new ones."""
        if self.box is None:
            return None
        size = self.box[2]
        k = (self.key, size)
        f = self.faces.get(k)
        if f is not None and f.source is self.img:
            return f
        if self.img is None:
            if self.app.covers.missing(self.key, size):  # no cover: the placeholder's colour
                f = self.faces[k] = flat_faces(size, rgb_of(self.theme.c("ph")))
                return f
            return None  # the cover is still on its way
        if k not in self.pending:
            self.pending.add(k)
            threading.Thread(target=self._make, args=(k, list(self.paths), self.img), daemon=True,
                             name="realhires-jacket").start()
            self._schedule_poll()
        return None

    def _make(self, k: tuple, paths: List[str], source) -> None:
        front = back = None
        try:
            png = covers.cached_cover_png(paths, k[1])
            rgb = covers.picture_rgb(png) if png else None
            if rgb is not None:
                front, back = rgb, covers.back_picture(rgb, rgb.shape[0], covers.BACK_SCRIM)
        except Exception:  # noqa: BLE001 - no pictures: the turn shows a plain sleeve
            front = back = None
        self.done.append((k, front, back, source))

    def _schedule_poll(self) -> None:
        if self._poll_job is None:
            try:
                self._poll_job = self.c.after(30, self._poll)
            except tk.TclError:
                pass

    def _poll(self) -> None:
        self._poll_job = None
        got = False
        while self.done:
            k, front, back, source = self.done.pop(0)
            self.pending.discard(k)
            if front is None or front.shape[0] != k[1] or front.shape[1] != k[1]:
                front = np.empty((k[1], k[1], 3), dtype=np.uint8)
                front[:] = rgb_of(self.theme.c("ph"))
                back = covers.darken(front, covers.BACK_SCRIM)
            self.faces.pop(k, None)
            while len(self.faces) >= self.MAX_KEPT:
                self.faces.pop(next(iter(self.faces)))
            self.faces[k] = Faces(front, back, source)
            got = True
        if self.pending:
            self._schedule_poll()
        if got:
            try:
                self.paint()
            except tk.TclError:
                pass

    def ready(self) -> bool:
        """The pictures of the cover shown are made (the tests wait for this)."""
        f = self.faces.get((self.key, self.box[2])) if self.box is not None else None
        return f is not None and f.source is self.img

    # ---- drawing -----------------------------------------------------------------
    def draw(self, x: float, y: float, size: int, img, key: str, paths: List[str], album: Optional[dict]) -> None:
        """Called by the page whenever it draws its block: the square at (x, y)."""
        self.box = (int(round(x)), int(round(y)), int(size))
        self.img, self.key, self.paths, self.album = img, key, list(paths), album
        if album is None and self.turn is not None:
            self._stop()
        if album is not None and album["key"] != self.first_key:
            # another album: from the top, with the song playing in view
            self.first_key, self.first, self.hover = album["key"], 0, None
            self.first = jacket.first_for(self._current(), 0, len(album["tracks"]), jacket.back_layout(size).rows)
        if img is not None and album is not None:
            self._faces()  # made ahead, so the first turn has its pictures
        self.paint()

    def _current(self) -> int:
        if self.album is None:
            return -1
        return next((i for i, t in enumerate(self.album["tracks"]) if t.path == self.page.path), -1)

    def side(self) -> str:
        """The face shown when nothing turns."""
        return self.app.np_side if self.album is not None else "cover"

    def _photo(self, rgb: np.ndarray) -> tk.PhotoImage:
        return tk.PhotoImage(master=self.c, data=to_ppm(rgb), format="PPM")

    def paint(self) -> None:
        """Draw the square again (the canvas items tagged "jacket")."""
        if self.box is None:
            return
        c = self.c
        c.delete("jacket")
        self._images = []
        self.page.song_rows = []
        x, y, size = self.box
        if self.turn is not None and self.album is not None:
            self._paint_turn()
        elif self.side() == "songs":
            self._paint_back()
        else:
            draw_cover(c, self.theme, self.img, x, y, size, tag="jacket")

    def _paint_turn(self) -> None:
        x, y, size = self.box
        turn = self.turn
        bg = rgb_of(self.theme.c("bg"))
        frame = jacket.turn_frame(turn["p"], turn["dir"])
        shown = turn["to"] if frame.other else turn["from"]
        faces = self._faces() or flat_faces(size, rgb_of(self.theme.c("ph")))
        front, back = faces.on(bg)[2:]
        img = self._photo(turn_picture(back if shown == "songs" else front, frame, bg))
        self._images.append(img)
        self.c.create_image(x, y, image=img, anchor="nw", tags=("jacket", "turn"))
        self.frames += 1
        self.last_frame = frame

    def _font(self, px: int, bold: bool = False) -> tkfont.Font:
        k = (px, bold, self.theme.family)
        f = self._fonts.get(k)
        if f is None:
            f = self._fonts[k] = tkfont.Font(self.c, family=self.theme.family, size=-px,
                                             weight="bold" if bold else "normal")
        return f

    def fade(self) -> float:
        """How far the back's text has faded in (0..1)."""
        if self.fade_t0 is None:
            return 1.0
        ms = jacket.fade_ms()
        return 1.0 if ms <= 0 else min(1.0, (time.monotonic() - self.fade_t0) * 1000.0 / ms)

    def _paint_back(self) -> None:
        """The back of the jacket: the album's title and artist, then its songs (number,
        title, time), inside the square, over the cover mirrored and blurred."""
        c, t = self.c, self.theme
        x, y, size = self.box
        album = self.album
        L = jacket.back_layout(size)
        bg = rgb_of(t.c("bg"))
        faces = self._faces() or flat_faces(size, rgb_of(t.c("ph")))
        back = faces.on(bg)[1]
        k = (self.key, size, bg, id(faces))
        if self._back_img is None or self._back_img[0] != k:
            self._back_img = (k, self._photo(back))
        c.create_image(x, y, image=self._back_img[1], anchor="nw", tags=("jacket", "back"))
        a = self.fade()
        white = hex_of(jacket.mix(faces.average, WHITE, a))
        grey = hex_of(jacket.mix(faces.average, WHITE, a * jacket.SECONDARY))
        fit: dict = {}
        title_f, small_f = self._font(L.title_px, True), self._font(L.small_px)
        body, bold = self._font(L.text_px), self._font(L.text_px, True)
        room = size - 2 * L.pad
        c.create_text(x + L.pad, y + L.title_y, text=fit_text(title_f, tr_name(album["title"]), room, fit),
                      anchor="w", fill=white, font=title_f, tags=("jacket", "back_title"))
        c.create_text(x + L.pad, y + L.artist_y, text=fit_text(small_f, tr_name(album["artist"]), room, fit),
                      anchor="w", fill=grey, font=small_f, tags=("jacket", "back_artist"))
        rows = list(album["tracks"])
        self.first = jacket.clamp_first(self.first, len(rows), L.rows)
        for n in range(L.rows):
            i = self.first + n
            if i >= len(rows):
                break
            r = rows[i]
            y0 = y + L.list_top + n * L.row_h
            mid = y0 + L.row_h / 2
            if i == self.hover and a >= 1.0:  # the row under the pointer: a white tint
                inset = max(3, L.pad // 2)
                crop = back[L.list_top + n * L.row_h + 1:L.list_top + (n + 1) * L.row_h - 1, inset:size - inset]
                tint = int(round(jacket.HOVER_TINT * 256))
                img = self._photo(((crop.astype(np.uint16) * (256 - tint) + 255 * tint) >> 8).astype(np.uint8))
                self._images.append(img)
                c.create_image(x + inset, y0 + 1, image=img, anchor="nw", tags=("jacket", "hover"))
            playing = r.path == self.page.path
            if playing:
                draw_icon(c, "speaker", x + L.num_right - L.text_px * 0.55, mid, L.text_px, white, tag="jacket")
            elif r.track:
                c.create_text(x + L.num_right, mid, text=str(r.track), anchor="e", fill=grey, font=body,
                              tags=("jacket",))
            when = fmt_time(r.duration) if r.duration else ""
            c.create_text(x + L.time_right, mid, text=when, anchor="e", fill=grey, font=body, tags=("jacket",))
            width = L.time_right - L.title_x - body.measure(when) - 10
            font = bold if playing else body
            c.create_text(x + L.title_x, mid, text=fit_text(font, r.display_title, width, fit), anchor="w",
                          fill=white, font=font, tags=("jacket", "song", "playing") if playing else ("jacket", "song"))
            self.page.song_rows.append((i, x, y0, x + size, y0 + L.row_h))
        off, length = jacket.scroll_bar(self.first, len(rows), L.rows, L.rows * L.row_h)
        if length:  # more songs than fit: a thin bar
            c.create_rectangle(x + L.bar_right - L.bar_w, y + L.list_top + off, x + L.bar_right,
                               y + L.list_top + off + length, fill=grey, outline="", tags=("jacket", "bar"))

    # ---- the turn ----------------------------------------------------------------
    def _stop(self) -> None:
        self.turn = None
        self.fade_t0 = None
        if self._job is not None:
            try:
                self.c.after_cancel(self._job)
            except tk.TclError:
                pass
            self._job = None

    def turn_to(self, side: str, direction: int = 0) -> None:
        """Turn the jacket to "cover" or "songs" (the dots, a sideways scroll, the keys).
        `direction`: which way the pointer went (0: to the songs as a drag to the left)."""
        app = self.app
        if side not in jacket.SIDES or side == app.np_side:
            return
        if not direction:
            direction = -1 if side == "songs" else 1
        old, ms = self.turn, jacket.flip_ms()
        app.np_side = side
        self.hover = None
        if ms <= 0 or self.album is None or self.box is None or not self.c.winfo_viewable():
            self._stop()
            self.page.draw_art()
            return
        if old is not None:  # turned back in the middle of a turn: from where it is
            p = 1.0 - old["p"] if old["to"] != side else old["p"]
            direction = -old["dir"] if old["to"] != side else old["dir"]
        else:
            p = 0.0
        self._start(jacket.other_side(side), side, direction, p, 1.0)

    def _start(self, frm: str, to: str, direction: int, p: float, target: float) -> None:
        ms = jacket.rest_ms(p, target >= 1.0, jacket.flip_ms())
        self.turn = {"from": frm, "to": to, "dir": direction, "p": p, "drag": False,
                     "anim": (p, target, time.monotonic(), ms)}
        self.fade_t0 = None
        self.page.draw_art()
        self._schedule()

    def _schedule(self) -> None:
        if self._job is None:
            try:
                self._job = self.c.after(FRAME_MS, self._tick)
            except tk.TclError:
                pass

    def _tick(self) -> None:
        self._job = None
        try:
            if not self.c.winfo_exists():
                return
            turn = self.turn
            if turn is not None and turn["anim"] is not None:
                p0, target, t0, ms = turn["anim"]
                elapsed = (time.monotonic() - t0) * 1000.0
                turn["p"] = jacket.progress_at(p0, target, elapsed, ms)
                if elapsed >= ms:
                    self.turn = None
                    if target >= 1.0 and turn["to"] == "songs" and jacket.fade_ms() > 0:
                        self.fade_t0 = time.monotonic()
                    self.page.draw_art()
                else:
                    self.paint()
            elif self.fade_t0 is not None:
                if self.fade() >= 1.0:
                    self.fade_t0 = None
                self.paint()
            if (self.turn is not None and self.turn["anim"] is not None) or self.fade_t0 is not None:
                self._schedule()
        except tk.TclError:
            pass

    def turning(self) -> bool:
        return self.turn is not None or self.fade_t0 is not None

    # ---- the pointer ---------------------------------------------------------------
    def contains(self, x: float, y: float) -> bool:
        if self.box is None:
            return False
        bx, by, size = self.box
        return bx <= x <= bx + size and by <= y <= by + size

    def pressed(self, e) -> None:
        self.press, self.dragged = (e.x, e.y), False

    def moved(self, e) -> None:
        """The pointer moved with the button down: a sideways drag turns the jacket with it."""
        if self.press is None:
            return
        dx, dy = e.x - self.press[0], e.y - self.press[1]
        if abs(dx) > DRAG_START_PX or abs(dy) > DRAG_START_PX:
            self.dragged = True
        if self.album is None or self.box is None:
            return
        turn = self.turn
        following = turn is not None and turn["drag"]
        if not following:
            if (turn is not None) or abs(dx) <= DRAG_START_PX or abs(dx) <= abs(dy):
                return
            self.fade_t0 = None
            self.hover = None
            side = self.app.np_side
            turn = self.turn = {"from": side, "to": jacket.other_side(side), "dir": 1, "p": 0.0, "drag": True,
                                "anim": None}
        turn["dir"] = 1 if dx > 0 else -1
        turn["p"] = jacket.drag_progress(dx, self.box[2])
        self.paint()

    def released(self, e) -> bool:
        """The button went up; True when the press was a drag (not a click)."""
        dragged, press = self.dragged, self.press
        self.press, self.dragged = None, False
        turn = self.turn
        if turn is not None and turn["drag"] and press is not None and self.box is not None:
            dx = e.x - press[0]
            finish = jacket.drag_finishes(dx, self.box[2])
            turn["drag"] = False
            if finish:
                self.app.np_side = turn["to"]
            if jacket.flip_ms() <= 0:
                self._stop()
                self.page.draw_art()
            else:
                self._start(turn["from"], turn["to"], turn["dir"], turn["p"], 1.0 if finish else 0.0)
        return dragged

    def pointer(self, x: float, y: float) -> None:
        """The pointer moved without a button: the row under it, and whether it is over the square."""
        self.over = self.contains(x, y)
        song = None
        if self.turn is None and self.side() == "songs":
            song = next((i for i, x0, y0, x1, y1 in self.page.song_rows if x0 <= x <= x1 and y0 <= y <= y1), None)
        if song != self.hover:
            self.hover = song
            self.paint()

    def left(self) -> None:
        self.over = False
        if self.hover is not None:
            self.hover = None
            self.paint()

    def wheel(self, n: int) -> None:
        """The wheel over the back of the jacket scrolls its songs."""
        if self.album is None or self.turn is not None or self.side() != "songs" or self.box is None:
            return
        rows = jacket.back_layout(self.box[2]).rows
        first = jacket.clamp_first(self.first + n * 3, len(self.album["tracks"]), rows)
        if first != self.first:
            self.first, self.hover = first, None
            self.paint()

    def key(self, direction: int) -> bool:
        """← / → while the pointer is over the jacket turn it (that way); True when it did."""
        if not self.over or self.album is None or self.box is None or self.press is not None:
            return False
        self.turn_to(jacket.other_side(self.app.np_side), direction)
        return True


# ----------------------------------------------------------------- menus ---
def search_google(app: "PlayerApp", path: str) -> str:
    """Search Cover Online → Google Images: the browser looks for the cover of the song's
    album; "" or what went wrong."""
    from .musiclib import Track

    t = app.track(path) or Track(path)
    return app.open_url(covers.cover_search_url(t.album_artist, t.artist, t.album, t.title or t.display_title))


def search_menu(app: "PlayerApp", parent: tk.Misc, path: str, on_error=None) -> tk.Menu:
    """Search Cover Online's two entries: MusicBrainz… (the cover search window) and
    Google Images (the browser)."""
    sub = tk.Menu(parent, tearoff=False)

    def google() -> None:
        error = search_google(app, path)
        if on_error is not None:
            on_error(error)

    sub.add_command(label=tr("MusicBrainz…"), command=lambda: app.open_cover_search(path))
    sub.add_command(label=tr("Google Images"), command=google)
    app.theme_menu(sub)
    return sub


def add_search_menu(app: "PlayerApp", m: tk.Menu, path: str) -> None:
    """Search Cover Online ▸ in a song's or an album's menu (2.5), after Change Cover…."""
    m.add_cascade(label=tr("Search Cover Online"), menu=search_menu(app, m, path))


def change_cover(app: "PlayerApp", path: str) -> None:
    """Change Cover… of a cover's own menu: the Cover window, and its file chooser at once."""
    w = app.open_cover_window(path)
    if w is not None:
        w.win.after(150, w.change)


def jacket_menu(app: "PlayerApp", path: str, go_album: bool = True) -> tk.Menu:
    """The right-click menu of a cover (2.5): Cover… (the Cover window), Change Cover…,
    Search Cover Online ▸ and, on Now Playing, Go to Album."""
    m = tk.Menu(app.root, tearoff=False)
    m.add_command(label=tr("Cover…"), command=lambda: app.open_cover_window(path))
    m.add_command(label=tr("Change Cover…"), command=lambda: change_cover(app, path))
    add_search_menu(app, m, path)
    track = app.track(path)
    album = app.album_of(track) if track is not None else None
    if go_album and album is not None:
        m.add_separator()
        m.add_command(label=tr("Go to Album"), command=lambda: app.open_album(album))
    app.theme_menu(m)
    app.last_menu = m
    return m


def bind_cover_menu(widget: tk.Misc, popup) -> None:
    """A right click (Button-3; Button-2 and Ctrl-click on macOS) calls popup(event)."""
    for seq in ("<Button-3>", "<Button-2>"):
        widget.bind(seq, popup)


class JacketMixin:
    """What the player app gains in 2.5 for covers: the cover search window, a cover's
    right-click menu and the arrow keys over Now Playing's jacket."""

    def open_cover_search(self: "PlayerApp", path: str):
        """Search Cover Online → MusicBrainz…: the Cover window of the song's album, and
        on top of it the window that finds covers on MusicBrainz (one at a time)."""
        from .player_coverart import CoverSearchWindow

        cw = self.open_cover_window(path)
        if cw is None or cw.path != path:
            return None  # no such song, or the Cover window is busy with another album
        w = getattr(self, "cover_search", None)
        if w is not None:
            try:
                alive = bool(w.win.winfo_exists())
            except tk.TclError:
                alive = False
            if alive and w.cw is cw:
                w.win.lift()
                w.win.focus_force()
                return w
            if alive:
                w.close()
        self.cover_search = CoverSearchWindow(self, cw)
        return self.cover_search

    def cover_menu(self: "PlayerApp", path: str, go_album: bool = True) -> tk.Menu:
        return jacket_menu(self, path, go_album)

    def jacket_key(self: "PlayerApp", e, direction: int) -> Optional[str]:
        """← / →: on Now Playing, with the pointer over the jacket, they turn it."""
        if self.qc.active or self._typing(e) or self.page_name != "nowplaying":
            return None
        p = self.pages.get("nowplaying")
        j = getattr(p, "jacket", None)
        return "break" if j is not None and j.key(direction) else None


__all__ = ["Jacket", "JacketMixin", "Faces", "turn_picture", "pack", "on_background", "corner_mask", "corner_radius", "jacket_menu",
           "add_search_menu", "search_menu", "search_google", "change_cover", "bind_cover_menu"]
