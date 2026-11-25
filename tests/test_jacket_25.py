"""The turning jacket and the cover search of 2.5 (docs/player-spec.md, "2.5", items 2
and 9): the back of the jacket (covers.back_picture(): mirrored, blurred, the same
size), the turn's geometry and the measures of the back (jacket.py), a frame of the
turn (player_jacket.turn_picture()), the Cover Art Archive client against a fake
archive (redirects, the size limit, one download at a time, never twice), what
MusicBrainz is asked, that the Rust vectors are current, and the Tk player headless in
a subprocess: the turn with its frames inside the cover's square, the back with the
album's songs, scrolling, the drag that follows the pointer, the keys, the cover's
menus and Search Cover Online → MusicBrainz… up to the cover set in the songs."""

import json
import os
import subprocess
import sys
import threading

import numpy as np
import pytest

import fake_coverart
import fake_musicbrainz
from player_library import make_library
from realhires import coverart, covers, jacket, musicbrainz
from test_player_gui import _gui_available, _has_miniaudio, needs_ffmpeg, run_script

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def halves(w=120, h=120):
    """Red on the left, blue on the right, under a fine black and white pattern."""
    a = np.zeros((h, w, 3), dtype=np.uint8)
    a[:, : w // 2, 0] = 200
    a[:, w // 2:, 2] = 200
    yy, xx = np.mgrid[0:h, 0:w]
    a[(xx + yy) % 2 == 0] += 40
    return a


def test_the_back_is_the_cover_mirrored_blurred_and_the_same_size():
    front = halves()
    back = covers.back_picture(front)
    assert back.shape == front.shape and back.dtype == np.uint8
    # mirrored left to right: the red is on the right now
    assert back[60, 105, 0] > 150 and back[60, 105, 2] < 60, back[60, 105]
    assert back[60, 15, 2] > 150 and back[60, 15, 0] < 60, back[60, 15]
    # clearly blurred: the fine pattern is gone and the edge between the halves is wide
    steps = np.abs(np.diff(back.astype(int), axis=1))
    assert steps.max() <= 14, steps.max()
    assert np.abs(np.diff(front.astype(int), axis=1)).max() >= 200
    assert int(((back[60, :, 0] > 40) & (back[60, :, 0] < 160)).sum()) >= 12
    assert np.abs(np.diff(back[20:30, 20:30].astype(int), axis=0)).max() <= 2
    # another size, and under the scrim half as bright
    assert covers.back_picture(front, 77).shape == (77, 77, 3)
    dark = covers.back_picture(front, 0, covers.BACK_SCRIM)
    assert np.array_equal(dark, back // 2)
    assert (covers.BACK_SMALL, covers.BACK_BLUR_RADIUS, covers.BACK_BLUR_PASSES, covers.BACK_SCRIM) == (64, 3, 3, 0.5)
    # any shape of picture gives a square back
    for shape in ((3, 5), (1, 1), (300, 90)):
        assert covers.back_picture(np.full(shape + (3,), 9, np.uint8), 40).shape == (40, 40, 3)


@needs_ffmpeg
def test_picture_rgb_reads_a_png(tmp_path):
    from realhires.imaging import to_png

    a = halves(40, 30)
    assert np.array_equal(covers.picture_rgb(to_png(a)), a)
    assert covers.picture_rgb(to_png(a), 16).shape == (16, 16, 3)
    assert covers.picture_rgb(b"not a picture") is None


def test_the_turn_narrows_to_the_edge_and_the_other_face_grows():
    assert (jacket.FLIP_MS, jacket.FADE_MS, jacket.SWIPE_PX) == (360, 120, 60)
    assert jacket.ease(0) == 0 and jacket.ease(1) == 1 and jacket.ease(0.5) == 0.5
    assert jacket.ease(0.1) < 0.1 and jacket.ease(0.9) > 0.9  # slow at both ends
    widths = [jacket.turn_frame(k / 20, 1).width for k in range(21)]
    assert widths[0] == 1 and abs(widths[10]) < 1e-9 and abs(widths[20] - 1) < 1e-9
    assert all(a >= b for a, b in zip(widths[:10], widths[1:11])) and all(a <= b for a, b in zip(widths[10:], widths[11:]))
    for d in (1, -1):
        first, half, second = jacket.turn_frame(0.25, d), jacket.turn_frame(0.5, d), jacket.turn_frame(0.75, d)
        assert not first.other and second.other, "the other face shows past the half-way point"
        assert first.far_right == (d > 0) and second.far_right == (d < 0), "the far edge crosses over"
        assert abs(half.dark - jacket.DARKEN) < 1e-9 and abs(half.far - (1 - jacket.PERSPECTIVE)) < 1e-9
        assert 0 < first.dark < jacket.DARKEN and 1 - jacket.PERSPECTIVE < first.far < 1
    assert jacket.turn_frame(0, 1).dark == 0 and jacket.turn_frame(0, 1).far == 1
    # a drag of the cover's width is a half turn; it finishes past 60 px or a quarter of the width
    assert jacket.drag_progress(160, 320) == 0.5 and jacket.drag_progress(-999, 320) == 1.0
    assert not jacket.drag_finishes(60, 480) and jacket.drag_finishes(-61, 480)
    assert not jacket.drag_finishes(40, 160) and jacket.drag_finishes(41, 160)
    assert jacket.rest_ms(0.25, True) == 270 and jacket.rest_ms(0.25, False) == 90
    assert jacket.progress_at(0.2, 1.0, 400, 360) == 1.0 and jacket.progress_at(0.2, 0.0, 0, 360) == 0.2


def test_the_back_holds_its_rows_inside_the_square():
    for size in (120, 160, 200, 240, 280, 320, 360, 400, 440, 480):
        L = jacket.back_layout(size)
        assert L.rows >= 1 and L.list_top + L.rows * L.row_h <= size - 4, L
        assert L.pad < L.num_right < L.title_x < L.time_right <= size - L.pad and L.bar_right <= size
        assert L.title_y < L.artist_y < L.list_top
    assert jacket.back_layout(480).rows > jacket.back_layout(160).rows
    # scrolling: never past the end, the song playing in view, a bar only when needed
    assert jacket.clamp_first(99, 20, 8) == 12 and jacket.clamp_first(-3, 20, 8) == 0 and jacket.clamp_first(5, 4, 8) == 0
    assert jacket.first_for(15, 0, 20, 8) == 8 and jacket.first_for(2, 5, 20, 8) == 2 and jacket.first_for(-1, 3, 20, 8) == 3
    assert jacket.scroll_bar(0, 8, 8, 240) == (0, 0)
    off, length = jacket.scroll_bar(12, 20, 8, 240)
    assert length == 96 and off + length == 240 and jacket.scroll_bar(0, 20, 8, 240)[0] == 0
    assert jacket.mix((0, 0, 0), (255, 255, 255), 0.5) == (128, 128, 128)


def test_the_animation_can_be_turned_off_for_tests(monkeypatch):
    monkeypatch.delenv(jacket.NO_ANIMATION_ENV, raising=False)
    assert (jacket.flip_ms(), jacket.fade_ms()) == (360, 120)
    monkeypatch.setenv(jacket.NO_ANIMATION_ENV, "1")
    assert (jacket.flip_ms(), jacket.fade_ms()) == (0, 0)
    monkeypatch.setenv(jacket.NO_ANIMATION_ENV, "0")
    assert jacket.flip_ms() == 360


def test_a_frame_of_the_turn_stays_inside_the_square():
    pytest.importorskip("tkinter")
    from realhires import player_jacket

    size, bg = 200, (10, 20, 30)
    face = player_jacket.on_background(np.full((size, size, 3), 200, np.uint8), bg)
    assert tuple(face[0, 0]) == bg and tuple(face[size // 2, 0]) == (200, 200, 200), "rounded corners"
    for d in (1, -1):
        for k in range(0, 21):
            f = jacket.turn_frame(k / 20, d)
            out = player_jacket.turn_picture(face, f, bg)
            assert out.shape == (size, size, 3) and out.dtype == np.uint8
            drawn = np.any(out != np.array(bg, np.uint8), axis=2)
            cols = np.flatnonzero(drawn.any(axis=0))
            want = max(2, round(size * f.width))
            assert abs(len(cols) - want) <= 2, (k, len(cols), want)
            assert abs((cols[0] + cols[-1]) / 2 - (size - 1) / 2) <= 1, "about the middle"
            mid = out[size // 2, size // 2, 0]
            assert abs(int(mid) - round(200 * (1 - f.dark))) <= 2, (k, mid)  # darkened while turned away
            if 0.3 < f.width < 0.999:
                left, right = drawn[:, cols[2]].sum(), drawn[:, cols[-3]].sum()
                assert (right < left) == f.far_right, (k, d, left, right)  # the far edge is shorter
    packed = player_jacket.pack(face)
    assert np.array_equal(player_jacket.turn_picture(packed, jacket.turn_frame(0.3, 1), bg),
                          player_jacket.turn_picture(face, jacket.turn_frame(0.3, 1), bg))


def test_cover_art_archive_one_download_at_a_time_and_never_twice(monkeypatch):
    monkeypatch.setenv("NO_PROXY", "127.0.0.1,localhost")
    monkeypatch.setenv("no_proxy", "127.0.0.1,localhost")
    fake = fake_coverart.FakeCoverArt(delay=0.05)
    try:
        monkeypatch.setenv(coverart.SERVER_ENV, fake.url)
        assert coverart.server() == fake.url
        c = coverart.CoverArtClient()
        thumb = c.front("rel-night-2001")
        assert thumb == fake_coverart.picture("rel-night-2001", True), "the redirect is followed"
        assert c.front("rel-night-2001", thumb=False) == fake_coverart.picture("rel-night-2001", False)
        assert c.front("rel-night-boot") is None, "no cover is not an error"
        # the archive asks to slow down: said once, not tried again
        fake.fail_next.append(503)
        with pytest.raises(coverart.CoverArtBusy, match="busy and asked to slow down"):
            c.front("rel-road-album")
        fake.fail_next.append(500)
        with pytest.raises(coverart.CoverArtError, match=r"could not send the picture \(HTTP 500\)"):
            c.front("rel-road-album")
        assert c.requests == 5
        assert fake.asked() == ["/release/rel-night-2001/front-250", "/release/rel-night-2001/front",
                                "/release/rel-night-boot/front-250", "/release/rel-road-album/front-250",
                                "/release/rel-road-album/front-250"]
        assert set(fake.agents) == {musicbrainz.user_agent()}
        # the size limit
        fake.big["rel-road-album"] = 5000
        small = coverart.CoverArtClient(fake.url, max_bytes=4000)
        with pytest.raises(coverart.CoverArtError, match="larger than"):
            small.front("rel-road-album", thumb=False)
        assert small.front("rel-road-album", thumb=True) is not None
        # two clients in two threads: still one download at a time
        fake.max_active = 0
        errors = []

        def many():
            try:
                mine = coverart.CoverArtClient(fake.url)
                for _ in range(3):
                    assert mine.front("rel-night-deluxe") is not None
            except Exception as exc:  # noqa: BLE001
                errors.append(exc)

        threads = [threading.Thread(target=many) for _ in range(2)]
        for t in threads:
            t.start()
        for t in threads:
            t.join(30)
        assert not errors and fake.max_active == 1, (errors, fake.max_active)
        # a stopped client asks for nothing
        stop = threading.Event()
        stop.set()
        n = len(fake.requests)
        with pytest.raises(coverart.CoverArtStopped):
            coverart.CoverArtClient(fake.url, stop=stop).front("rel-night-2001")
        assert len(fake.requests) == n
    finally:
        fake.close()
    # something that is not a picture, a server that is not there
    text = coverart.CoverArtClient("http://x.invalid", download=lambda *a: (200, {}, b"<html>"))
    with pytest.raises(coverart.CoverArtError, match="not a picture"):
        text.front("a")

    def gone(*a):
        raise OSError("no route to host")

    with pytest.raises(coverart.CoverArtError, match="Could not reach x.invalid: no route to host."):
        coverart.CoverArtClient("http://x.invalid", download=gone).front("a")
    assert coverart.DEFAULT_SERVER == "https://coverartarchive.org" and coverart.MAX_BYTES == 20 * 1024 * 1024
    assert coverart.front_url("a b/c", True, "https://coverartarchive.org/") == \
        "https://coverartarchive.org/release/a%20b%2Fc/front-250"


def test_musicbrainz_is_asked_for_the_album_and_its_artist_only():
    seen = []
    client = musicbrainz.MusicBrainzClient("https://musicbrainz.org", transport=fake_musicbrainz.transport(seen),
                                           min_interval=0.0, clock=lambda: 0.0, sleep=lambda s: None)
    found = coverart.find_releases(client, "Night Drive", "The Example Band")
    assert [r.id for r in found] == ["rel-night-2001", "rel-night-deluxe", "rel-night-boot"]
    assert [coverart.release_facts(r) for r in found] == ["2001 · JP · CD", "2015 · XW · 2×CD", "2003 · Cassette"]
    assert seen == ["https://musicbrainz.org/ws/2/release?query=release%3A%22Night+Drive%22+AND+artist%3A%22"
                    "The+Example+Band%22&limit=12&fmt=json"]
    assert coverart.MAX_RELEASES == 12
    assert coverart.search_terms("Band", "Singer", "Album", "Song") == ("Album", "Band")
    assert coverart.search_terms("", "Singer", "", " A  Song ") == ("A Song", "Singer")
    assert "Cover Art Archive" in coverart.CREDIT and "MusicBrainz" in coverart.CREDIT
    # both apps show the credit in the window and in About
    assert "coverart.CREDIT" in open(os.path.join(ROOT, "realhires", "player_coverart.py"), encoding="utf-8").read()
    assert "coverart::CREDIT" in open(os.path.join(ROOT, "rust", "crates", "gui", "src", "coversearch.rs"),
                                      encoding="utf-8").read()
    assert "coverart_credit()" in open(os.path.join(ROOT, "realhires", "player_gui.py"), encoding="utf-8").read()
    assert "coverart::ABOUT_CREDIT" in open(os.path.join(ROOT, "rust", "crates", "gui", "src", "app.rs"), encoding="utf-8").read()


def test_the_rust_vectors_are_current(tmp_path):
    out = tmp_path / "jacket.json"
    subprocess.run([sys.executable, os.path.join(ROOT, "rust", "tools", "gen_jacket_vectors.py"), str(out)],
                   check=True, capture_output=True)
    committed = os.path.join(ROOT, "rust", "crates", "core", "tests", "python_jacket.json")
    new, old = json.loads(out.read_text(encoding="utf-8")), json.load(open(committed, encoding="utf-8"))
    assert new["covers"] == old["covers"], "run rust/tools/gen_jacket_vectors.py (the back's pixels)"
    assert new["coverart"] == old["coverart"], "run rust/tools/gen_jacket_vectors.py"
    assert _close(new["jacket"], old["jacket"]), "run rust/tools/gen_jacket_vectors.py"


def _close(a, b) -> bool:
    """Equal JSON, floats within rounding (another CPU gives other last digits of sin and cos)."""
    if isinstance(a, float) or isinstance(b, float):
        return isinstance(a, (int, float)) and isinstance(b, (int, float)) and abs(a - b) <= 1e-12 * max(1, abs(a), abs(b))
    if isinstance(a, dict) and isinstance(b, dict):
        return a.keys() == b.keys() and all(_close(a[k], b[k]) for k in a)
    if isinstance(a, list) and isinstance(b, list):
        return len(a) == len(b) and all(_close(x, y) for x, y in zip(a, b))
    return a == b


@needs_ffmpeg
@pytest.mark.skipif(not _has_miniaudio(), reason="miniaudio not installed")
@pytest.mark.skipif(not _gui_available(), reason="needs Tkinter and a display")
def test_the_jacket_turns_over_and_covers_come_from_musicbrainz(tmp_path):
    lib = make_library(tmp_path, seconds=3, mp3=False)
    # an album with more songs than fit on the back of its jacket; its cover is yellow on
    # the left and dark blue on the right
    folder = tmp_path / "Music" / "The Long Band" / "Long Player"
    folder.mkdir(parents=True)
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-f", "lavfi", "-i",
                    "color=c=0x101060:s=300x300:d=1,drawbox=x=0:y=0:w=150:h=300:color=0xf0d020:t=fill",
                    "-frames:v", "1", str(folder / "cover.png")], check=True)
    long_songs = []
    for n in range(1, 21):
        path = folder / f"{n:02d} Long Song {n}.flac"
        subprocess.run(["ffmpeg", "-v", "error", "-y", "-f", "lavfi", "-i", f"sine=frequency={200 + 10 * n}:sample_rate=8000:"
                        "duration=4", "-metadata", f"title=Long Song {n}", "-metadata", "artist=The Long Band",
                        "-metadata", "album=Long Player", "-metadata", f"track={n}/20", str(path)], check=True)
        long_songs.append(str(path))
    from realhires import settings

    mb, caa = fake_musicbrainz.FakeMusicBrainz(), fake_coverart.FakeCoverArt(delay=0.03)
    try:
        cfg = settings.Settings()
        cfg.library_folders = [lib["music"]]
        cfg.theme = "Light"
        cfg.musicbrainz_server = mb.url
        cfg.analyse_moods = False
        settings.save(cfg, str(tmp_path / "settings.json"))
        env = dict(os.environ, REALHIRES_SETTINGS=str(tmp_path / "settings.json"),
                   REALHIRES_PLAYER=str(tmp_path / "player.json"), REALHIRES_LIBRARY=str(tmp_path / "library.sqlite"),
                   REALHIRES_RESULTS=str(tmp_path / "results.sqlite"), REALHIRES_THEMES=str(tmp_path / "themes.json"),
                   REALHIRES_COVERS=str(tmp_path / "covers"), REALHIRES_TABS=str(tmp_path / "tabs"),
                   REALHIRES_AUDIO_BACKEND="null", NO_PROXY="127.0.0.1,localhost", no_proxy="127.0.0.1,localhost")
        env.pop(jacket.NO_ANIMATION_ENV, None)  # the turn takes its time here
        env[coverart.SERVER_ENV] = caa.url
        env[musicbrainz.INTERVAL_ENV] = "0"
        script = _SCRIPT.replace("@LIB@", json.dumps(lib)).replace("@LONG@", json.dumps(long_songs))
        run_script(tmp_path, script.replace("@N@", str(len(lib["songs"]) + 20)), env, timeout=200)
        # what was asked: the album's artist and title of MusicBrainz; the releases' ids of the
        # archive, each once, one at a time, by name; never a path or a song
        # (the window searches at once when it opens: twice for Long Player, once for what was typed)
        assert len(mb.requests) == 3 and all("limit=12" in r for r in mb.requests), mb.requests
        assert mb.requests[2] == mb.requests[0]
        assert "release%3A%22Long+Player%22+AND+artist%3A%22The+Long+Band%22" in mb.requests[0]
        assert "release%3A%22Night+Drive%22+AND+artist%3A%22The+Example+Band%22" in mb.requests[1]
        assert caa.asked() == ["/release/rel-night-2001/front-250", "/release/rel-night-deluxe/front-250",
                               "/release/rel-night-boot/front-250", "/release/rel-night-2001/front"], caa.requests
        assert caa.max_active == 1, "downloads overlapped"
        assert set(caa.agents) | set(mb.agents) == {musicbrainz.user_agent()}
        assert not any("Music" in r or "flac" in r or "Song" in r for r in mb.requests + caa.requests)
    finally:
        mb.close()
        caa.close()


_SCRIPT = """
LIB = @LIB@
LONG = @LONG@
S = LIB["songs"]
import numpy as np
import tkinter as tk
import tkinter.font as tkfont
import fake_coverart
from realhires import covers, jacket, player_gui

def fail(exc, val, tb):
    import traceback
    traceback.print_exception(exc, val, tb)
    os._exit(1)

def wait(cond, then, timeout=60.0, what="", start=None):
    start = start or time.monotonic()
    if cond():
        return root.after(50, then)
    assert time.monotonic() - start < timeout, "timed out: " + what
    root.after(40, lambda: wait(cond, then, timeout, what, start))

def settle():
    for _ in range(3):
        root.update()

def entries(m):
    return [m.entrycget(i, "label") if m.type(i) != "separator" else "-" for i in range(m.index("end") + 1)]

def texts(tag):
    return [np_.art.itemcget(i, "text") for i in np_.art.find_withtag(tag) if np_.art.type(i) == "text"]

def click(widget, x, y):
    widget.event_generate("<ButtonPress-1>", x=x, y=y)
    widget.event_generate("<ButtonRelease-1>", x=x, y=y)
    settle()

def square():
    x, y, s = j.box
    return x, y, x + s, y + s

def inside(tag="jacket"):
    # everything the jacket draws is inside the cover's square
    bb = np_.art.bbox(tag)
    x0, y0, x1, y1 = square()
    return bb is not None and bb[0] >= x0 - 1 and bb[1] >= y0 - 1 and bb[2] <= x1 + 1 and bb[3] <= y1 + 1

def weight(item):
    return tkfont.Font(root=root, font=np_.art.itemcget(item, "font")).actual("weight")

class Ev:
    def __init__(self, widget):
        self.widget = widget

root = tk.Tk()
root.geometry("1280x800")
root.report_callback_exception = fail
app = player_gui.PlayerApp(root)
opened = []
app.url_opener = opened.append  # never a real browser
np_ = app.page("nowplaying")
j = np_.jacket
state = {"frames": []}
paint = j.paint
def watched():
    paint()
    if j.turn is not None and np_.art.find_withtag("turn"):
        assert inside(), ("a frame of the turn leaves the square", np_.art.bbox("jacket"), square())
        assert not texts("song"), "the faces are pictures only during the turn"
        state["frames"].append(j.last_frame)
j.paint = watched

def loaded():
    album = next(a for a in app.albums if a["title"] == "Long Player")
    state["album"] = album
    app.play_list(album["tracks"], 2, "Long Player")
    app.engine.pause()
    settle()
    app.cover_clicked()
    settle()
    assert app.page_name == "nowplaying" and app.np_side == "cover"
    wait(j.ready, front, what="the jacket's pictures")

def front():
    # the cover, with its two dots; its pictures: the cover and its back, the same size
    x0, y0, x1, y1 = square()
    cover = next(l for l in np_.links if l[0] == "cover")
    assert all(abs(a - b) <= 1 for a, b in zip(cover[1:5], (x0, y0, x1, y1))), (cover, square())
    assert [d[0] for d in np_.dots] == ["cover", "songs"] and not texts("song")
    f = j.faces[(j.key, j.box[2])]
    assert f.front.shape == f.back.shape == (j.box[2], j.box[2], 3)
    w = f.front.shape[1]
    left, right = f.front[:, : w // 4].mean(axis=(0, 1)), f.back[:, -(w // 4):].mean(axis=(0, 1))
    assert np.abs(left / 2 - right).max() < 25, ("mirrored, under the scrim", left, right)
    assert np.abs(np.diff(f.back.astype(int), axis=1)).max() <= 12, "blurred"
    assert np.abs(np.diff(f.front.astype(int), axis=1)).max() > 100
    # a click on the cover opens the album; the Cover window is on its right-click menu
    cx, cy = (x0 + x1) // 2, (y0 + y1) // 2
    state["c"] = (cx, cy)
    click(np_.art, cx, cy)
    assert app.page_name == "album" and app.page("album").album["title"] == "Long Player"
    assert getattr(app, "cover_window", None) is None
    page = app.page("album")
    m = page.cover_menu()
    assert entries(m) == ["Cover…", "Change Cover…", "Search Cover Online"], entries(m)  # no Go to Album here
    w = page.open_cover()  # a click on the album page's cover: the Cover window
    assert w is not None and w is app.cover_window and len(w.target_paths()) == 20
    assert str(page.cover.cget("cursor")) == "hand2"
    w.close()
    app.go_back()
    settle()
    assert app.page_name == "nowplaying"
    m = np_.cover_menu()
    assert entries(m) == ["Cover…", "Change Cover…", "Search Cover Online", "-", "Go to Album"], entries(m)
    sub = root.nametowidget(m.entrycget(2, "menu"))
    assert entries(sub) == ["MusicBrainz…", "Google Images"], entries(sub)
    sub.invoke(1)
    assert opened == ["https://www.google.com/search?tbm=isch&q=The+Long+Band+Long+Player"], opened
    m.invoke(0)
    settle()
    assert app.cover_window is not None and app.cover_window.path == LONG[2]
    app.cover_window.close()
    m.invoke(4)
    settle()
    assert app.page_name == "album"
    app.go_back()
    settle()
    # the song and album menus gained Search Cover Online
    labels = entries(app.song_menu(app.track(LONG[2])))
    i = labels.index("Change Cover…")
    assert labels[i + 1] == "Search Cover Online", labels
    # a dot turns the jacket over: the side is chosen at once, the turn shows it
    state["n"] = j.frames
    side, a, b, c, d = np_.dots[1]
    click(np_.art, int((a + c) / 2), int((b + d) / 2))
    assert app.np_side == "songs" and j.turning(), (app.np_side, j.turn)
    wait(lambda: not j.turning(), back, what="the turn")

def back():
    frames = state["frames"]
    assert j.frames > state["n"] and frames, "no frame of the turn was drawn"
    others = [f.other for f in frames]
    assert others == sorted(others), ("first the cover, then its back", others)
    if len(frames) >= 6:  # (a machine that drew this many saw the jacket get narrow on the way)
        assert min(f.width for f in frames) < 0.9 and True in others and False in others, frames
    # the back: inside the square, the album's songs in white over the blurred cover
    art = np_.art
    assert art.find_withtag("back") and not art.find_withtag("turn") and inside()
    L = jacket.back_layout(j.box[2])
    titles = texts("song")
    assert L.rows < 20 and len(titles) == L.rows and titles[0] == "Long Song 1", (L.rows, titles)
    assert len(np_.song_rows) == L.rows
    assert texts("back_title") == ["Long Player"] and texts("back_artist") == ["The Long Band"]
    assert weight(art.find_withtag("back_title")[0]) == "bold"
    playing = art.find_withtag("playing")
    assert len(playing) == 1 and art.itemcget(playing[0], "text") == "Long Song 3" and weight(playing[0]) == "bold"
    other = next(i for i in art.find_withtag("song") if i not in playing)
    assert weight(other) == "normal" and art.itemcget(other, "fill") == "#ffffff"
    assert art.itemcget(art.find_withtag("back_artist")[0], "fill") != "#ffffff", "secondary texts at 70 %"
    bar = art.find_withtag("bar")
    assert len(bar) == 1, "more songs than fit: a thin bar"
    y_bar = art.coords(bar[0])[1]
    # the wheel scrolls inside the square
    cx, cy = state["c"]
    if root.tk.call("tk", "windowingsystem") == "x11":
        art.event_generate("<Button-5>", x=cx, y=cy)
    else:
        art.event_generate("<MouseWheel>", x=cx, y=cy, delta=-120)
    settle()
    assert j.first == 3 and texts("song")[0] == "Long Song 4" and inside(), (j.first, texts("song"))
    assert art.coords(art.find_withtag("bar")[0])[1] > y_bar
    j.wheel(99)
    assert texts("song")[-1] == "Long Song 20" and j.first == 20 - L.rows and inside()
    j.wheel(-99)
    assert j.first == 0
    # the row under the pointer: a white tint
    i, x0, y0, x1, y1 = np_.song_rows[4]
    art.event_generate("<Motion>", x=int(x0 + 60), y=int((y0 + y1) / 2))
    settle()
    assert j.hover == 4 and len(art.find_withtag("hover")) == 1 and inside() and j.over
    # a click on the back does nothing; a double-click plays that song with the album as the list
    click(art, int(x0 + 60), int((y0 + y1) / 2))
    assert app.page_name == "nowplaying" and app.current == LONG[2]
    click(art, int(x0 + 60), int((y0 + y1) / 2))
    app.engine.pause()
    settle()
    assert app.current == LONG[4] and app.context_name == "Long Player", app.current
    assert art.itemcget(art.find_withtag("playing")[0], "text") == "Long Song 5"
    root.after(500, drag)  # (later, or Tk counts the next press with these as a triple click)

def drag():
    # a sideways drag turns the jacket with the pointer; let go early and it springs back
    art, (cx, cy), size = np_.art, state["c"], j.box[2]
    art.event_generate("<ButtonPress-1>", x=cx, y=cy)
    art.event_generate("<B1-Motion>", x=cx + 30, y=cy + 2)
    settle()
    assert j.turn is not None and j.turn["drag"] and abs(j.turn["p"] - 30 / size) < 0.01, j.turn
    assert art.find_withtag("turn") and inside() and app.np_side == "songs", "30 px is not a turn yet"
    art.event_generate("<B1-Motion>", x=cx + int(size * 0.7), y=cy + 3)
    settle()
    assert abs(j.turn["p"] - int(size * 0.7) / size) < 0.01 and j.last_frame.other and inside()
    art.event_generate("<B1-Motion>", x=cx + 30, y=cy + 2)
    art.event_generate("<ButtonRelease-1>", x=cx + 30, y=cy + 2)
    settle()
    assert app.np_side == "songs", "let go at 30 px: the jacket springs back"
    wait(lambda: not j.turning(), drag2, what="the jacket to spring back")

def drag2():
    art, (cx, cy), size = np_.art, state["c"], j.box[2]
    assert app.np_side == "songs" and art.find_withtag("back") and texts("song")
    art.event_generate("<ButtonPress-1>", x=cx, y=cy)
    art.event_generate("<B1-Motion>", x=cx - 100, y=cy)
    settle()
    assert j.turn["dir"] == -1 and app.np_side == "songs", "not before the pointer lets go"
    art.event_generate("<ButtonRelease-1>", x=cx - 100, y=cy)
    settle()
    assert app.np_side == "cover" and app.page_name == "nowplaying", "let go past 60 px: it turns over; a drag is no click"
    wait(lambda: not j.turning(), keys, what="the turn")

def keys():
    art, (cx, cy) = np_.art, state["c"]
    assert not texts("song") and not art.find_withtag("back")
    # the arrow keys turn it while the pointer is over it, and only then
    assert root.bind("<Left>") and root.bind("<Right>")
    art.event_generate("<Motion>", x=5, y=5)
    assert not j.over and app.jacket_key(Ev(art), 1) is None and app.np_side == "cover"
    art.event_generate("<Motion>", x=cx, y=cy)
    assert j.over and app.jacket_key(Ev(art), 1) == "break" and app.np_side == "songs"
    assert j.turn["dir"] == 1
    wait(lambda: not j.turning(), keys2, what="the turn")

def keys2():
    art = np_.art
    assert app.jacket_key(Ev(art), -1) == "break" and app.np_side == "cover" and j.turn["dir"] == -1
    # turned again in the middle of the turn: it goes back from where it is
    j.turn["p"] = 0.3
    assert app.jacket_key(Ev(art), -1) == "break" and app.np_side == "songs"
    assert abs(j.turn["p"] - 0.7) < 1e-9 and j.turn["dir"] == 1, j.turn
    wait(lambda: not j.turning(), small, what="the turn")

def small():
    # in a small window the back still fits its square, and the square its half
    assert texts("song")
    root.geometry("900x600")
    settle()
    wait(j.ready, small2, what="the pictures at the new size")

def small2():
    settle()
    x0, y0, x1, y1 = square()
    assert inside() and x0 >= 0 and y0 >= 0 and x1 <= np_.art.winfo_width() and y1 <= np_.art.winfo_height()
    L = jacket.back_layout(j.box[2])
    assert len(texts("song")) == min(L.rows, 20) and len(np_.song_rows) == len(texts("song"))
    app.set_np_side("cover")
    # Search Cover Online → MusicBrainz…: the Cover window, and the cover search on top of it
    w = app.open_cover_search(LONG[0])
    state["w"] = w
    assert w is app.cover_search and app.cover_window is not None and w.cw is app.cover_window
    assert (w.v_album.get(), w.v_artist.get()) == ("Long Player", "The Long Band")
    assert app.open_cover_search(LONG[0]) is w, "one at a time"
    wait(lambda: w.done and not w.searching, nothing, what="the first search")

def nothing():
    w = state["w"]
    assert w.status.cget("text").startswith("MusicBrainz knows no release called “Long Player”"), w.status.cget("text")
    w.v_album.set("Night Drive")
    w.v_artist.set("The Example Band")
    w.search()
    wait(lambda: w.done and not w.searching, found, what="the covers")

def found():
    w = state["w"]
    assert [t["release"].id for t in w.tiles] == ["rel-night-2001", "rel-night-deluxe"] and len(w.releases) == 3
    assert w.status.cget("text") == "2 covers found. Click one to see it large.", w.status.cget("text")
    c = w.canvas
    shown = [c.itemcget(i, "text") for i in c.find_all() if c.type(i) == "text"]
    assert shown == ["Night Drive", "The Example Band", "2001 · JP · CD", "Night Drive", "The Example Band",
                     "2015 · XW · 2×CD"], shown
    assert len(c.find_withtag("thumb")) == 2 and len(w.boxes) == 2
    assert all(b[1] >= 0 and b[3] <= c.winfo_width() for b in w.boxes), (w.boxes, c.winfo_width())
    assert not w.use_b.winfo_ismapped() and "Cover Art Archive" in w.credit.cget("text")
    # a click shows it large with its size in pixels
    i, x0, y0, x1, y1 = w.boxes[0]
    c.event_generate("<ButtonRelease-1>", x=int((x0 + x1) / 2), y=int((y0 + y1) / 2))
    settle()
    assert w.chosen is not None and w.chosen.id == "rel-night-2001"
    wait(lambda: w.picture is not None, large, what="the chosen cover")

def large():
    w = state["w"]
    settle()
    assert w.details.cget("text").startswith("600 × 600 pixels · PNG"), w.details.cget("text")
    assert w.caption.cget("text") == "Night Drive — The Example Band  ·  2001 · JP · CD"
    assert w.use_b.winfo_ismapped() and w.back_b.winfo_ismapped() and w.canvas.find_withtag("large")
    assert w.picture == fake_coverart.picture("rel-night-2001", False)
    w.back()
    settle()
    assert w.chosen is None and len(w.canvas.find_withtag("thumb")) == 2 and not w.use_b.winfo_ismapped()
    w.choose(0)  # chosen again: it is not downloaded again
    assert w.picture == fake_coverart.picture("rel-night-2001", False) and not w.loading
    wait(lambda: w.can_use(), use, what="the chosen cover again")

def use():
    w, cw = state["w"], app.cover_window
    state["cw"] = cw
    assert cw.scope.get() == "album"
    # Use this cover: the Cover window sets it as Change cover… does; this window closes
    assert w.use() and app.cover_search is None and cw.busy
    wait(lambda: not cw.busy and not cw.reading and not app.library_busy(), done, what="the covers changed")

def done():
    cw = state["cw"]
    assert cw.summary_text == "Changed the cover of 20 songs.", cw.summary_text
    for p in (LONG[0], LONG[7], LONG[19]):
        src = covers.cover_source([p])
        assert src.kind == "embedded" and src.data == fake_coverart.picture("rel-night-2001", False), p
    # the cover search goes with its Cover window
    w = app.open_cover_search(LONG[0])
    assert w is not None and app.cover_search is w
    cw.close()
    settle()
    assert app.cover_window is None and app.cover_search is None
    print("OK")
    app.quit()

wait(lambda: len(app.tracks) == @N@ and not app.library_busy(), loaded, what="the library")
root.mainloop()
"""
