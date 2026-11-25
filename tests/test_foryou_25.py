"""For You (2.5, docs/player-spec.md "1. For You: playlists made from likes, retention
and listening time").

The core (`realhires/recommend.py`): the formula on invented listening histories (a song
skipped after ten seconds every time sinks, a liked song heard to the end rises, a song
heard a lot a year ago gives way to one heard a lot this month, no mix is dominated by
one artist), the mixes' rules, and the vectors of the Rust port.

The Tk player: the For You entries of the sidebar and the Library page, the For You
page's cards at 1024 x 768, a mix's page (Play, Shuffle, Save as Playlist: the
playlist's songs and order, a taken name gets " 2"), the mixes made again after a play
is logged and after a like and a dislike, and the texts in Japanese.  The histories are
written straight into the test's library file.  The GUI tests run in a subprocess, as
test_player_gui.py's (skipped without Tk or a display; locally: xvfb-run -a python -m
pytest)."""

import json
import math
import os
import re
import shutil
import subprocess
import sys

import pytest

from player_library import make_library
from test_player_gui import _gui_available, _has_miniaudio, run_script
from realhires import recommend as R
from realhires import settings
from realhires.musiclib import Track
from realhires.stats import Play

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
needs_ffmpeg = pytest.mark.skipif(shutil.which("ffmpeg") is None, reason="ffmpeg not installed")
NOW = 1795600000.0
DAY = 86400.0


def play(path, days_ago, seconds, duration=240.0):
    return Play(path, NOW - days_ago * DAY, float(seconds), float(duration))


def stat(plays, liked=(), disliked=()):
    return R.song_stats(plays, liked, disliked, NOW).get("s", R.SongStats("s"))


# ------------------------------------------------------------------ the formula ---
def test_a_song_skipped_early_every_time_sinks():
    scores = [stat([play("s", 1 + k / 5, 10) for k in range(n)]).score for n in (1, 5, 20, 100, 400)]
    assert all(s < -0.5 for s in scores), scores  # however often: skipping never adds up to liking
    assert scores[1] < scores[0]  # and more skips sink it further
    # with the first formula (every second counts) a hundred skips made a favourite
    skipped = [play("s", 1 + k / 5, 10) for k in range(100)]
    first = 4 * (stat(skipped).retention - 0.5) + math.log2(1 + sum(R.weight(NOW - p.started) * p.seconds
                                                                   for p in skipped) / 180)
    assert first > 0.5 > 0 > stat(skipped).score
    # even liked, it stays far below a liked song that is heard to the end
    assert stat(skipped, liked=["s"]).score < 1 < stat([play("s", 1, 240)], liked=["s"]).score


def test_a_liked_song_heard_to_the_end_rises():
    heard = [play("s", 1 + k, 240) for k in range(5)]
    assert stat(heard, liked=["s"]).score == pytest.approx(stat(heard).score + R.LIKE_POINTS)
    assert stat(heard).score > stat(heard[:1]).score > stat([]).score == 0.0
    assert stat([], liked=["s"]).score == R.LIKE_POINTS  # liked, never played
    assert stat([], disliked=["s"]).score == -R.LIKE_POINTS
    assert stat(heard).retention > 0.8 and stat(heard).plays == 5 and stat(heard).skips == 0
    half = [play("s", 1 + k, 120) for k in range(5)]
    assert stat(heard).score > stat(half).score > 0  # half heard is worth less than heard to the end


def test_old_listening_gives_way_to_new():
    year_ago = [play("s", 365 + k, 240) for k in range(20)]
    this_month = [play("s", 1 + k, 240) for k in range(20)]
    assert stat(this_month).score > stat(year_ago).score + 4 and stat(year_ago).score > 0
    assert stat(year_ago).retention < 0.6 < 0.9 < stat(this_month).retention  # old plays fade to "not known"
    # the plain numbers do not fade: what the user thought of the song when they played it
    assert stat(year_ago).plain_score == pytest.approx(stat(this_month).plain_score)
    assert stat(year_ago).plain_retention == stat(this_month).plain_retention == (20 + 1) / (20 + 2)
    assert R.weight(60 * DAY) == pytest.approx(0.5) and R.weight(0.0) == R.weight(-5.0) == 1.0


def test_plays_without_a_duration_and_counts():
    # a play of a song without a known duration is whole after 30 s
    assert R.heard(15, 0) == 0.5 and R.heard(45, 0) == 1.0 and R.heard(120, 240) == 0.5 and R.heard(500, 240) == 1.0
    s = stat([play("s", 1, 40, 0), play("s", 2, 5, 0), play("s", 40, 240, 240), play("s", -3, 100, 240)])
    # (without a duration, stats.counted() takes a second as a play)
    assert (s.plays, s.skips, s.seconds, s.recent_plays, s.recent_seconds) == (4, 0, 385.0, 3, 145.0)
    assert s.last_played == NOW + 3 * DAY
    assert R.key(1.0) == 1000000 and R.key(-4.9e-7) == 0 and R.key(5.1e-7) == 1
    assert sorted(R.song_stats([], ["a"], ["b"], NOW)) == ["a", "b"]


# -------------------------------------------------------------------- the mixes ---
def invented_library():
    """Eight artists with six songs and one with thirty, and a listening history."""
    tracks = [Track(f"/m/A{a}/{s:02d}.flac", title=f"A{a} song {s}", artist=f"Artist {a}", album=f"Album {a}",
                    genre="Rock" if a < 4 else "Jazz", duration=240.0) for a in range(8) for s in range(6)]
    tracks += [Track(f"/m/Big/{s:02d}.flac", title=f"Big {s}", artist="Big Band", album=f"Big {s // 10}",
                     genre="Pop", duration=200.0) for s in range(30)]
    plays, liked, disliked = [], set(), set()
    for s in range(20):   # Big Band: the favourite, twenty songs heard to the end six times this month
        plays += [play(f"/m/Big/{s:02d}.flac", 1 + k * 4 + s * 0.1, 200, 200) for k in range(6)]
    for s in range(4):    # Artist 0: liked and heard to the end
        liked.add(f"/m/A0/{s:02d}.flac")
        plays += [play(f"/m/A0/{s:02d}.flac", 2 + k * 3, 240) for k in range(5)]
    for s in range(6):    # Artist 1: skipped after ten seconds every time
        plays += [play(f"/m/A1/{s:02d}.flac", 1 + k * 2, 10) for k in range(8)]
    for s in range(5):    # Artist 2: heard a lot a year ago
        plays += [play(f"/m/A2/{s:02d}.flac", 350 + k, 240) for k in range(20)]
    for s in range(3):    # Artist 3: heard a lot this month
        plays += [play(f"/m/A3/{s:02d}.flac", 1 + k, 240) for k in range(20)]
    liked.update(f"/m/A4/{s:02d}.flac" for s in range(2))  # Artist 4: liked, never played
    disliked.add("/m/A5/00.flac")                          # Artist 5: one song disliked
    plays.append(play("/m/A6/00.flac", 3, 240))            # Artist 6: one song heard once; Artist 7: nothing
    plays += [play("/m/gone.flac", 1 + k, 240) for k in range(9)]  # a favourite whose file is gone
    plays.sort(key=lambda p: p.started)
    return tracks, plays, liked, disliked


def test_the_mixes_of_an_invented_library():
    tracks, plays, liked, disliked = invented_library()
    by = {t.path: t for t in tracks}
    mixes = {m["id"]: m for m in R.mixes(tracks, plays, liked, disliked, NOW, "2026-11-25")}
    assert list(mixes) == list(R.MIX_IDS) and [m["name"] for m in mixes.values()] == R.mix_names()
    stats = R.song_stats(plays, liked, disliked, NOW)
    for mid, m in mixes.items():
        paths = m["paths"]
        assert len(paths) >= R.MIN_SONGS and len(set(paths)) == len(paths), mid
        assert all(p in by and p not in disliked for p in paths), mid  # never a song that is gone or disliked
        artists = [by[p].artist for p in paths]
        limit = R.DAILY_PER_ARTIST if mid == "daily" else R.MAX_PER_ARTIST
        most = max(artists.count(a) for a in set(artists))
        assert most <= limit or len(paths) <= R.MIN_FILL, (mid, artists)  # no artist dominates a mix
        if most <= limit and len(set(artists)) > 2:
            assert all(a != b for a, b in zip(artists, artists[1:])), (mid, artists)  # and none follows itself
        assert not any(by[p].artist == "Artist 1" for p in paths), mid  # the skipped songs are in no mix
    fav = mixes["favourites"]["paths"]
    assert by[fav[0]].artist == "Artist 3"  # heard twenty times this month
    assert max(i for i, p in enumerate(fav) if by[p].artist == "Artist 3") < \
        min(i for i, p in enumerate(fav) if by[p].artist == "Artist 2")  # this month before a year ago
    assert {"/m/A4/00.flac", "/m/A4/01.flac"} <= set(fav)  # liked but never played
    assert [by[p].artist for p in fav].count("Big Band") == 4  # twenty favourites of one artist: four of them
    assert all(R.key(stats[p].score) > 0 for p in fav)
    assert {by[p].artist for p in mixes["on_repeat"]["paths"]} == {"Artist 0", "Artist 3", "Artist 6", "Big Band"}
    assert {by[p].artist for p in mixes["rediscover"]["paths"]} == {"Artist 2"}  # all five: the limit gives way
    assert len(mixes["rediscover"]["paths"]) == 5
    new = mixes["discover"]["paths"]
    assert all(p not in stats or not stats[p].plays for p in new)  # never played
    assert by[new[0]].artist == "Artist 3"  # the rest of the artist the user listens to most
    assert any(by[p].artist == "Artist 7" for p in new)  # nothing known but the genre the user likes
    assert not any(by[p].artist == "Artist 5" for p in new)  # one disliked song speaks against the artist
    # the Daily Mix: another one each day, the same one all day, the other mixes do not move
    days = [R.mixes(tracks, plays, liked, disliked, NOW, d) for d in ("2026-11-25", "2026-11-26", "2026-11-25")]
    assert days[0] == days[2] == list(mixes.values())
    assert days[0][-1]["paths"] != days[1][-1]["paths"] and days[0][:-1] == days[1][:-1]
    assert R.day_of(NOW) in ("2026-11-24", "2026-11-25", "2026-11-26") and R.mixes(tracks, plays, liked, disliked, NOW)


def test_small_libraries_and_the_limit_per_artist():
    tracks, plays, liked, disliked = invented_library()
    # one artist only: the limit gives way until a mix has MIN_FILL songs
    solo = [t for t in tracks if t.artist == "Big Band"]
    got = {m["id"]: len(m["paths"]) for m in R.mixes(solo, plays, liked, disliked, NOW, "2026-11-25")}
    assert got == {"favourites": 12, "on_repeat": 12, "discover": 10, "daily": 12}, got
    # fewer than MIN_SONGS songs: no mix; nothing played or liked: no mix
    assert R.mixes(tracks[:4], [play(t.path, 1, 240) for t in tracks[:4]], [], [], NOW, "2026-11-25") == []
    assert R.mixes(tracks, [], [], [], NOW, "2026-11-25") == []
    assert R.mixes([], plays, liked, disliked, NOW, "2026-11-25") == []
    # the limit keeps the order, songs without an artist are not limited
    names = {"a1": "a", "a2": "a", "a3": "a", "b1": "b", "u1": "", "u2": ""}
    assert R.limit_artists(["a1", "a2", "u1", "a3", "u2", "b1"], names.get, 50, 1) == ["a1", "a2", "u1", "a3", "u2",
                                                                                      "b1"]  # under MIN_FILL: all
    many = [f"a{i}" for i in range(30)] + [f"b{i}" for i in range(30)]
    assert R.limit_artists(many, lambda p: p[0], 50, 4) == many[:6] + many[30:36]  # 4 + 4, then up to MIN_FILL
    assert R.spread_artists(["a1", "a2", "a3", "b1", "u1"], names.get) == ["a1", "b1", "a2", "u1", "a3"]
    assert R.spread_artists(["b1", "a1", "a2", "a3"], names.get)[0] == "a1"  # or two of them would meet
    tagged = Track("/x", artist="Björk feat. Someone", album=" Homogenic ", album_artist="", genre=" Pop ")
    assert (R.artist_key(tagged), R.album_key(tagged), R.genre_key(tagged)) == (
        "björk", "homogenic\x00björk feat. someone", "pop")
    assert R.artist_key(Track("/y", album_artist="Various")) == "various" and R.artist_key(Track("/z")) == ""


def test_a_song_skipped_again_and_again_is_not_new():
    tracks = [Track(f"/m/{i}.flac", artist="Loved" if i < 8 else f"Other {i}", genre="Pop", duration=240.0)
              for i in range(12)]
    plays = [play(t.path, 1 + k, 240) for t in tracks[:5] for k in range(4)]  # five songs of Loved, heard a lot
    once = plays + [play("/m/5.flac", 1, 10)]
    often = once + [play("/m/6.flac", 1 + k, 10) for k in range(3)]
    new = R.candidates(tracks, often, [], [], NOW)["discover"]
    assert "/m/5.flac" in new and "/m/7.flac" in new and "/m/6.flac" not in new, new
    assert new.index("/m/7.flac") < new.index("/m/5.flac")  # one skip costs a few places
    assert new[:3] == ["/m/7.flac", "/m/5.flac", "/m/10.flac"]  # then the genre's other songs, by path


# -------------------------------------------------------------- the Rust port ---
def test_the_rust_port_has_the_same_constants_and_texts():
    with open(os.path.join(ROOT, "rust", "crates", "core", "src", "recommend.rs"), encoding="utf-8") as fh:
        rust = fh.read()
    for name in ("HALF_LIFE_DAYS", "PRIOR_R", "PRIOR_N", "NO_DURATION_SECONDS", "LIKE_POINTS", "RETENTION_POINTS",
                 "TIME_UNIT", "MIN_SONGS", "MAX_PER_ARTIST", "DAILY_PER_ARTIST", "MIN_FILL", "FAVOURITES_SIZE",
                 "ON_REPEAT_SIZE", "ON_REPEAT_DAYS", "REDISCOVER_SIZE", "REDISCOVER_DAYS", "REDISCOVER_PLAYS",
                 "REDISCOVER_RETENTION", "DISCOVER_SIZE", "DISCOVER_SKIPS", "DAILY_SIZE", "DAILY_REDISCOVER",
                 "DAILY_DISCOVER", "DAILY_POOL"):
        assert re.search(rf"pub const {name}: \w+ = {re.escape(repr(getattr(R, name)))};", rust), name
    for mid, name, text in R.MIXES:
        assert f'("{mid}", "{name}", "{text}")' in rust, mid
    assert R.EMPTY_NOTE in rust and R.HOW_NOTE in rust.replace('=\n    "', '= "')


def test_the_rust_vectors_are_current(tmp_path):
    out = tmp_path / "recommend.json"
    subprocess.run([sys.executable, os.path.join(ROOT, "rust", "tools", "gen_recommend_vectors.py"), str(out)],
                   check=True, capture_output=True)
    committed = os.path.join(ROOT, "rust", "crates", "core", "tests", "python_recommend.json")
    # numbers within rounding (another C library gives other last digits of 0.5 ** x and log2);
    # every order, name and count exactly
    with open(committed, encoding="utf-8") as fh:
        diff = _first_difference(json.loads(out.read_text(encoding="utf-8")), json.load(fh))
    assert diff is None, f"run rust/tools/gen_recommend_vectors.py: {diff}"


def test_the_vectors_cover_what_they_should():
    with open(os.path.join(ROOT, "rust", "crates", "core", "tests", "python_recommend.json"), encoding="utf-8") as fh:
        data = json.load(fh)
    big = next(s for s in data["scenarios"] if s["name"] == "big")
    stats = big["stats"].values()
    in_library = {t["path"] for t in data["libraries"]["big"]}
    assert sum(1 for s in stats if s["liked"] and not s["plays"] and not s["skips"]) >= 5   # liked, never played
    assert sum(1 for s in stats if s["skips"] >= 3 and not s["plays"]) >= 5                 # skipped again and again
    assert sum(1 for s in stats if s["plays"] >= 3 and s["last_played"] < NOW - 200 * DAY) >= 5   # long ago
    assert sum(1 for s in stats if s["recent_plays"] >= 3) >= 5                             # and recently
    assert sum(1 for s in stats if s["disliked"]) >= 5
    assert sum(1 for s in stats if s["path"] not in in_library) == 3                        # files that are gone
    assert sum(1 for p in data["histories"]["big"] if p["duration"] == 0) >= 5              # no known duration
    assert len(big["days"]) == 4 and all(len(ms) == 5 for ms in big["days"].values())
    daily = [ms[-1]["paths"] for ms in big["days"].values()]
    assert all(daily[0] != d for d in daily[1:])
    gone = {s["path"] for s in stats} - in_library
    assert not any(gone & set(m["paths"]) for ms in big["days"].values() for m in ms)
    ties = next(s for s in data["scenarios"] if s["name"] == "ties")
    assert ties["candidates"]["favourites"][:8] == sorted(ties["candidates"]["favourites"][:8])  # the path decides
    assert len({ties["stats"][p]["score"] for p in ties["candidates"]["favourites"][:8]}) == 1
    solo = next(s for s in data["scenarios"] if s["name"] == "one artist")
    assert [len(m["paths"]) for m in next(iter(solo["days"].values()))][:2] == [R.MIN_FILL, R.MIN_FILL]


def _first_difference(a, b, path="$"):
    """Where two JSON values differ (floats within 1e-9 relative), or None."""
    if isinstance(a, bool) or isinstance(b, bool):
        return None if a is b else f"{path}: {a!r} != {b!r}"
    if isinstance(a, float) or isinstance(b, float):
        if isinstance(a, (int, float)) and isinstance(b, (int, float)) and math.isclose(a, b, rel_tol=1e-9,
                                                                                        abs_tol=1e-9):
            return None
        return f"{path}: {a!r} != {b!r}"
    if isinstance(a, dict) and isinstance(b, dict):
        if a.keys() != b.keys():
            return f"{path}: keys {sorted(set(a) ^ set(b))} differ"
        return next((d for d in (_first_difference(a[k], b[k], f"{path}.{k}") for k in a) if d), None)
    if isinstance(a, list) and isinstance(b, list):
        if len(a) != len(b):
            return f"{path}: {len(a)} items != {len(b)}"
        return next((d for d in (_first_difference(x, y, f"{path}[{i}]") for i, (x, y) in enumerate(zip(a, b))) if d),
                    None)
    return None if a == b else f"{path}: {a!r} != {b!r}"


# ------------------------------------------------------------------ the player ---
def _env(tmp_path, music, **values):
    cfg = settings.Settings()
    cfg.library_folders = [music]
    cfg.theme = "Light"
    cfg.analyse_moods = False  # not this test's business
    for k, v in values.items():
        setattr(cfg, k, v)
    settings.save(cfg, str(tmp_path / "settings.json"))
    return dict(os.environ, REALHIRES_SETTINGS=str(tmp_path / "settings.json"),
                REALHIRES_PLAYER=str(tmp_path / "player.json"), REALHIRES_LIBRARY=str(tmp_path / "library.sqlite"),
                REALHIRES_RESULTS=str(tmp_path / "results.sqlite"), REALHIRES_THEMES=str(tmp_path / "themes.json"),
                REALHIRES_COVERS=str(tmp_path / "covers"), REALHIRES_TABS=str(tmp_path / "tabs"),
                REALHIRES_AUDIO_BACKEND="null")


HELPERS = """
import dataclasses
import tkinter as tk
from realhires import musiclib, player_foryou, player_gui, recommend
from realhires.i18n import tr
player_gui.messagebox.askyesno = lambda *a, **k: True
player_gui.messagebox.showinfo = lambda *a, **k: None

def fail(exc, val, tb):
    import traceback
    traceback.print_exception(exc, val, tb)
    os._exit(1)

def wait(cond, then, timeout=90.0, what="", start=None):
    start = start or time.monotonic()
    if cond():
        return root.after(50, then)
    assert time.monotonic() - start < timeout, "timed out: " + what
    root.after(100, lambda: wait(cond, then, timeout, what, start))

def texts(canvas):
    return [canvas.itemcget(i, "text") for i in canvas.find_all() if canvas.type(i) == "text"]

def click(canvas, box):
    x, y = (box[0] + box[2]) / 2 - canvas.canvasx(0), (box[1] + box[3]) / 2 - canvas.canvasy(0)
    canvas.event_generate("<Motion>", x=int(x), y=int(y))
    canvas.event_generate("<Button-1>", x=int(x), y=int(y))
    root.update()

def press(button):
    button.event_generate("<ButtonRelease-1>", x=5, y=5)
    root.update()

def invent_history():
    # the invented listening history, written straight into the library file (the songs are
    # a few seconds long; the history says four minutes, as real songs are)
    hist = musiclib.Library()
    now = time.time()

    def log(title, days_ago, seconds, n=1):
        t = dataclasses.replace(app.by_path[S[title]], duration=240.0)
        for k in range(n):
            assert hist.add_play(t, now - (days_ago + k) * 86400.0, seconds)

    log("First Light", 1, 240, 5)       # Aurora Fields: heard to the end this week
    log("Glacier Song", 1, 240, 5)
    log("Midnight Sun", 1, 240, 3)
    log("Polar Drift", 1, 240, 2)
    log("Umbrella Weather", 1, 10, 6)   # skipped after ten seconds every time
    log("Taxi Lights", 2, 240, 2)
    log("Blue Kettle", 100, 240, 4)     # heard to the end long ago
    log("Carrier Wave", 1, 240, 2)      # heard, and disliked
    log("Low Pass", 1, 240)
    hist.close()                        # Last Train Home and Porch Lamp: never played

root = tk.Tk()
root.geometry("1024x740+0+0")  # the CI runners' 1024 x 768 screens
root.report_callback_exception = fail
app = player_gui.PlayerApp(root)
"""


@needs_ffmpeg
@pytest.mark.skipif(not _has_miniaudio(), reason="miniaudio not installed")
@pytest.mark.skipif(not _gui_available(), reason="needs Tkinter and a display")
def test_for_you_in_the_player(tmp_path):
    lib = make_library(tmp_path, seconds=3, mp3=False)
    env = _env(tmp_path, lib["music"])
    body = f"""
S = {json.dumps(lib["songs"])}
""" + HELPERS + """
def loaded():
    assert len(app.tracks) == 11
    # the sidebar's Playlists section starts with For You, the Library page has it after Playlists
    items = app.sidebar.items()
    keys = [k for k, _, _ in items]
    assert keys[keys.index("foryou") - 1] == "head" and keys[keys.index("foryou") + 1] == "liked", keys
    assert ("foryou", "For You", "sparkle") in items
    page = app.page("library")
    entries = page.entries()
    i = [e[0] for e in entries].index("foryou")
    assert entries[i - 1][0] == "playlists" and entries[i + 1][0] == "liked"
    assert entries[i][1:4] == ("For You", "sparkle", "No mixes yet"), entries[i]
    # the icon is drawn (a vector shape, in the sidebar and on the tile)
    assert len(app.sidebar.nav.find_withtag("icon")) > len(keys) - 5
    click(page.c, next(box for box, key in page.entry_hits if key == "foryou"))
    assert app.page_name == "foryou" and app.sidebar.selected == "foryou"
    fy = app.page("foryou")
    shown = texts(fy.c)
    assert shown == ["For You", recommend.HOW_NOTE, recommend.EMPTY_NOTE], shown
    assert fy.card_hits == {} and app.foryou_mixes() == []

    # ---- an invented history, a like and a dislike: three mixes
    invent_history()
    app.foryou_changed()
    app.toggle_like(S["First Light"])
    app.toggle_like(S["Porch Lamp"])
    app.toggle_dislike(S["Carrier Wave"])
    root.update()
    mixes = app.foryou_mixes()
    assert [m["id"] for m in mixes] == ["favourites", "on_repeat", "daily"], [m["id"] for m in mixes]
    want = recommend.mixes(app.tracks, app.plays(), app.state.liked, app.state.disliked, time.time())
    assert mixes == want
    fav = mixes[0]["paths"]
    title = {p: t for t, p in S.items()}
    names = [title[p] for p in fav]
    print("favourites", names, flush=True)
    assert names[0] == "First Light"  # liked and heard to the end five times
    assert set(names) == {"First Light", "Glacier Song", "Midnight Sun", "Polar Drift", "Taxi Lights", "Blue Kettle",
                          "Porch Lamp", "Low Pass"}, names
    assert names.index("Glacier Song") < names.index("Blue Kettle")  # this week before a hundred days ago
    artists = [app.by_path[p].artist for p in fav]
    assert all(a != b for a, b in zip(artists, artists[1:])), artists  # no artist twice in a row
    for m in mixes:  # the skipped and the disliked song are in no mix
        assert S["Umbrella Weather"] not in m["paths"] and S["Carrier Wave"] not in m["paths"], m["id"]
    assert set(title[p] for p in mixes[1]["paths"]) == {"First Light", "Glacier Song", "Midnight Sun", "Polar Drift",
                                                        "Taxi Lights", "Low Pass"}
    assert S["Last Train Home"] in mixes[2]["paths"]  # never played, by an artist the user listens to

    # ---- the For You page: a card per mix, inside a 1024 px window
    shown = texts(fy.c)
    for m in mixes:
        assert m["name"] in shown and m["description"] in shown, (m["name"], shown)
    assert shown[:2] == ["For You", recommend.HOW_NOTE] and recommend.EMPTY_NOTE not in shown
    assert shown.count("8 songs") == 1 and "6 songs" in shown, shown
    assert list(fy.card_hits) == ["favourites", "on_repeat", "daily"]
    width = fy.c.winfo_width()
    boxes = list(fy.card_hits.values())
    assert all(0 <= b[0] and b[2] <= width for b in boxes), (boxes, width)
    assert boxes[0][1] == boxes[1][1] == boxes[2][1] and boxes[0][2] < boxes[1][0] < boxes[1][2] < boxes[2][0]
    app.show_page("library")
    root.update()
    assert dict((e[0], e[3]) for e in app.page("library").entries())["foryou"] == "3 mixes"
    app.go_back()
    root.update()
    assert app.page_name == "foryou"

    # ---- a mix's page
    click(fy.c, fy.card_hits["favourites"])
    assert app.page_name == "foryou_mix" and app.sidebar.selected == "foryou"
    mp = app.page("foryou_mix")
    assert mp.mid == "favourites" and mp.title_l.cget("text") == "Favourites Mix"
    assert mp.note_l.cget("text") == mixes[0]["description"] and mp.meta_l.cget("text") == "8 songs, 0 minutes"
    assert [r.path for r in mp.table.rows] == fav
    assert (mp.play_b.text, mp.shuffle_b.text, mp.save_b.text) == ("Play", "Shuffle", "Save as Playlist")
    assert mp.save_b.winfo_rootx() + mp.save_b.winfo_width() <= root.winfo_rootx() + 1024
    assert app.page_label("foryou_mix", app._page_args()) == "Favourites Mix"
    heads = texts(mp.table.canvas)
    assert "Title" in heads and "1" in heads and "8" in heads, heads  # the place in the mix, then the columns

    # ---- Save as Playlist: the mix's name, songs and order; again: " 2"
    press(mp.save_b)
    assert [p["name"] for p in app.state.playlists] == ["Favourites Mix"]
    assert app.state.playlists[0]["songs"] == fav
    assert mp.made.cget("text") == app.foryou_message == "Made the playlist “Favourites Mix” with 8 songs."
    assert app.page_name == "foryou_mix"  # it stays on the mix
    press(mp.save_b)
    assert [p["name"] for p in app.state.playlists] == ["Favourites Mix", "Favourites Mix 2"]
    assert mp.made.cget("text") == "Made the playlist “Favourites Mix 2” with 8 songs."
    assert ("playlist:" + app.state.playlists[0]["id"]) in [k for k, _, _ in app.sidebar.items()]
    saved = json.load(open(os.environ["REALHIRES_PLAYER"], encoding="utf-8"))
    assert saved["playlists"][0]["songs"] == fav

    # ---- Play: the mix from its first song; then the next song logs the play and the mixes are made again
    press(mp.play_b)
    assert app.current == fav[0] and app.context_name == "Favourites Mix", (app.current, app.context_name)
    wait(lambda: app.listen is not None and app.listen["seconds"] >= 1.2, played, what="a second of the first song")

def played():
    mp = app.page("foryou_mix")
    before, ver, logged = app.foryou_mixes(), app._foryou_ver, len(app.plays())
    fav = [r.path for r in mp.table.rows]
    app.next()
    root.update()
    assert app.current == fav[1]
    assert len(app.plays()) == logged + 1 and app._foryou_ver == ver + 1
    after = app.foryou_mixes()
    assert after is not before and [m["id"] for m in after] == ["favourites", "on_repeat", "daily"]
    assert [r.path for r in mp.table.rows] == after[0]["paths"]  # the page shows the mix made again
    press(mp.shuffle_b)
    assert app.queue.shuffle != "off" and app.current in after[0]["paths"]
    app.engine.pause()
    app.set_shuffle("off")

    # ---- a dislike takes the song out at once, a like brings one in
    gone = after[0]["paths"][2]
    app.toggle_dislike(gone)
    root.update()
    rows = [r.path for r in mp.table.rows]
    assert gone not in rows and len(rows) == 7 and mp.meta_l.cget("text") == "7 songs, 0 minutes", rows
    app.toggle_like(S["Last Train Home"])
    root.update()
    rows = [r.path for r in mp.table.rows]
    assert S["Last Train Home"] in rows and len(rows) == 8, rows
    app.toggle_dislike(gone)  # undone
    root.update()
    assert gone in [r.path for r in mp.table.rows]

    # ---- Back returns to For You; the other mixes open too
    app.go_back()
    root.update()
    assert app.page_name == "foryou"
    fy = app.page("foryou")
    click(fy.c, fy.card_hits["daily"])
    mp = app.page("foryou_mix")
    assert mp.mid == "daily" and mp.title_l.cget("text") == "Daily Mix" and mp.made.cget("text") == ""
    assert [r.path for r in mp.table.rows] == app.foryou_mix("daily")["paths"]
    assert len(mp.table.rows) >= 5

    # ---- the history cleared: the three likes are too few for a mix of favourites, but their
    # artists' other songs are New For You; without the likes nothing is left
    app.clear_history(confirm=False)
    root.update()
    left = app.foryou_mixes()
    assert [m["id"] for m in left] == ["discover", "daily"], [m["id"] for m in left]
    assert S["Glacier Song"] in left[0]["paths"] and S["First Light"] not in left[0]["paths"]
    assert [r.path for r in mp.table.rows] == left[1]["paths"]
    for name in ("First Light", "Porch Lamp", "Last Train Home"):
        app.toggle_like(S[name])
    root.update()
    assert app.state.liked == []
    assert app.foryou_mixes() == [] and mp.table.rows == [] and not mp.save_b.enabled
    assert mp.save() is None
    app.show_page("foryou")
    root.update()
    assert recommend.EMPTY_NOTE in texts(fy.c) and fy.card_hits == {}
    print("OK")
    app.quit()

wait(lambda: len(app.tracks) == 11 and not app.library_busy(), loaded, what="the library")
root.mainloop()
"""
    run_script(tmp_path, body, env)

    # ---- the same library in Japanese, with the history again
    env = _env(tmp_path, lib["music"], language="日本語")
    body = f"""
S = {json.dumps(lib["songs"])}
""" + HELPERS + """
def check():
    invent_history()
    app.foryou_changed()
    app.toggle_like(S["Porch Lamp"])  # seven favourites by listening, and this one
    items = [text for _, text, _ in app.sidebar.items()]
    assert "おすすめ" in items, items
    app.show_page("foryou")
    root.update()
    fy = app.page("foryou")
    shown = texts(fy.c)
    for want in ("おすすめ", "お気に入りミックス", "ヘビーローテーション", "デイリーミックス", "8曲"):
        assert want in shown, (want, shown)
    assert not any(s in shown for s in (recommend.HOW_NOTE, "Favourites Mix", "For You")), shown
    click(fy.c, fy.card_hits["favourites"])
    mp = app.page("foryou_mix")
    assert mp.title_l.cget("text") == "お気に入りミックス" and mp.save_b.text == "プレイリストとして保存"
    assert mp.meta_l.cget("text") == "8曲、0分", mp.meta_l.cget("text")
    assert mp.save_b.winfo_rootx() + mp.save_b.winfo_width() <= root.winfo_rootx() + 1024
    press(mp.save_b)
    assert app.state.playlists[-1]["name"] == "お気に入りミックス", app.state.playlists[-1]["name"]
    assert mp.made.cget("text") == "8曲のプレイリスト「お気に入りミックス」を作成しました。", mp.made.cget("text")
    app.show_page("library")
    root.update()
    assert "3個のミックス" in texts(app.page("library").c)
    print("OK")
    app.quit()

wait(lambda: len(app.tracks) == 11 and not app.library_busy(), check, what="the library")
root.mainloop()
"""
    run_script(tmp_path, body, env)
