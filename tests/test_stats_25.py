"""2.5 (docs/player-spec.md, "2.5" items 4 and 5): the cards of Your Stats lead on, the
leaderboard sorted by times played or time listened, its kind Genres, Most Played's own
period control (Day, Week, Month, Year, All time, with ‹ and ›), and the quality tooltip
with the bit rate and sample rate.  The core is checked here and against the Rust
vectors; the Tk pages run in a subprocess at 1024 × 768, as test_stats_24.py's do
(skipped without Tk or a display; locally: xvfb-run -a python -m pytest)."""

import json
import shutil
import time
from pathlib import Path

import pytest

from player_library import make_library
from test_player_gui import _gui_available, _has_miniaudio, _has_mp3_encoder, run_script
from test_stats_24 import HELPERS, _env
from realhires import i18n, musiclib, stats
from realhires.musiclib import Track
from realhires.stats import Play

needs_ffmpeg = pytest.mark.skipif(shutil.which("ffmpeg") is None, reason="ffmpeg not installed")
TESTS = Path(__file__).resolve().parents[1] / "rust" / "crates" / "core" / "tests"


def _play(path, started, seconds, title="", artist="", album="", genre="", duration=200.0):
    return Play(path, started, seconds, duration, title, artist, album, "", genre)


# ------------------------------------------------------------------------ core ---
def test_leaderboard_by_time_listened_and_genres():
    plays = [
        _play("/m/long.flac", 1, 900, "Long", "Ann", "Slow", "Ambient", 900),          # 1 play, 15 min
        _play("/m/often.flac", 2, 100, "Often", "Ann", "Slow", "ambient", 100),       # 3 plays, 5 min
        _play("/m/often.flac", 3, 100, "Often", "Ann", "Slow", "ambient", 100),
        _play("/m/often.flac", 4, 100, "Often", "Ann", "Slow", "ambient", 100),
        _play("/m/twice.flac", 5, 150, "Twice", "Bob", "Pairs", "Rock", 150),         # 2 plays, 5 min
        _play("/m/twice.flac", 6, 150, "Twice", "Bob", "Pairs", "Rock", 150),
        _play("/m/a.flac", 7, 120.4, "Same A", "Cy", "Twins"),                        # 1 play, 2 min
        _play("/m/b.flac", 8, 119.5, "same b", "Cy", "Twins"),                        # 1 play, 2 min: the same rank
        _play("/m/skip.flac", 9, 29, "Skipped", "Dee", "Skips", "Rock", 400),         # never counted: no row
        _play("/m/skip.flac", 10, 29, "Skipped", "Dee", "Skips", "Rock", 400),
    ]
    by_plays = stats.leaderboard(plays, "songs")
    assert by_plays == stats.leaderboard(plays, "songs", by="plays")
    assert [(r["rank"], r["name"]) for r in by_plays] == [
        (1, "Often"), (2, "Twice"), (3, "Long"), (4, "Same A"), (4, "same b")]
    by_time = stats.leaderboard(plays, "songs", by="time")
    assert [(r["rank"], r["name"], r["minutes"], r["plays"]) for r in by_time] == [
        (1, "Long", 15, 1), (2, "Often", 5, 3), (3, "Twice", 5, 2), (4, "Same A", 2, 1), (4, "same b", 2, 1)]
    assert sorted(r["key"] for r in by_time) == sorted(r["key"] for r in by_plays), "the same rows, another order"
    # an artist's songs: the most played first, or the longest listened first
    assert stats.leaderboard(plays, "artists")[0]["paths"] == ["/m/often.flac", "/m/long.flac"]
    assert stats.leaderboard(plays, "artists", by="time")[0]["paths"] == ["/m/long.flac", "/m/often.flac"]
    assert stats.ranked_songs(by_time)[:2] == ["/m/long.flac", "/m/often.flac"]
    # the kind Genres: the genre's name (its first spelling), no artist, uncounted plays' time included
    genres = stats.leaderboard(plays, "genres")
    assert [(r["rank"], r["key"], r["name"], r["artist"], r["plays"], r["minutes"]) for r in genres] == [
        (1, "ambient", "Ambient", "", 4, 20), (2, "rock", "Rock", "", 2, 6),
        (3, "unknown genre", "Unknown Genre", "", 2, 4)]
    assert genres[1]["paths"] == ["/m/twice.flac", "/m/skip.flac"]
    assert stats.LEADERBOARD_KINDS == ["songs", "artists", "albums", "genres"]
    assert stats.LEADERBOARD_SORTS == ["plays", "time"]
    assert len(stats.leaderboard(plays, "songs", top=2, by="time")) == 2
    with pytest.raises(ValueError):
        stats.leaderboard(plays, "songs", by="length")


def test_spans_of_the_period_control():
    today = (2026, 11, 25)  # a Wednesday
    assert stats.span_days("day", 0, today) == ((2026, 11, 25), (2026, 11, 26))
    assert stats.span_days("day", -25, today) == ((2026, 10, 31), (2026, 11, 1))
    assert stats.span_days("week", 0, today) == ((2026, 11, 23), (2026, 11, 30))  # Monday to Sunday
    assert stats.span_days("week", 0, (2026, 11, 23)) == stats.span_days("week", 0, (2026, 11, 29))
    assert stats.span_days("week", -1, today) == ((2026, 11, 16), (2026, 11, 23))
    assert stats.span_days("month", 0, today) == ((2026, 11, 1), (2026, 12, 1))
    assert stats.span_days("month", -11, today) == ((2025, 12, 1), (2026, 1, 1))
    assert stats.span_days("year", -1, today) == ((2025, 1, 1), (2026, 1, 1))
    assert stats.span_days("all", -3, today) is None
    assert [stats.span_name(u, 0, today) for u in stats.SPAN_UNITS] == [
        "25 November 2026", "23 – 29 November 2026", "November 2026", "2026", "All time"]
    assert stats.span_name("week", 1, today) == "30 November – 6 December 2026"
    assert stats.span_name("week", 5, today) == "28 December 2026 – 3 January 2027"
    assert stats.span_name("day", -1, (2024, 3, 1)) == "29 February 2024"
    # the range: between local midnights (here a clock that is always UTC)
    utc, back = time.gmtime, __import__("calendar").timegm
    now = back((2026, 11, 25, 15, 30, 0, 0, 0, 0))
    midnight = back((2026, 11, 25, 0, 0, 0, 0, 0, 0))
    assert stats.span_range("day", 0, now, utc, back) == (midnight, midnight + 86400)
    assert stats.span_range("day", -1, now, utc, back) == (midnight - 86400, midnight)
    assert stats.span_range("week", 0, now, utc, back) == (midnight - 2 * 86400, midnight + 5 * 86400)
    assert stats.span_range("month", 0, now, utc, back) == (midnight - 24 * 86400, midnight + 6 * 86400)
    assert stats.span_range("year", 0, now, utc, back)[0] == stats.period_range("This year", now, utc, back)[0]
    assert stats.span_range("all", 0, now, utc, back) == (0.0, None)
    assert stats.span_title("week", -1, now, utc) == "16 – 22 November 2026"
    # opened from Your Stats, Most Played starts from that page's period
    assert [stats.period_span(p) for p in stats.PERIODS] == [("month", 0), ("year", 0), ("year", -1), ("all", 0)]


def test_quality_tooltip_names_the_bit_rate():
    hi = Track("/a.flac", codec="flac", sample_rate=96000, bits=24, bit_rate=2304000)
    cd = Track("/b.flac", codec="flac", sample_rate=44100, bits=16, bit_rate=1411200)
    fl = Track("/c.wav", codec="pcm_f32le", sample_rate=192000, bits=32, is_float=True, bit_rate=12288000)
    mp3 = Track("/d.mp3", codec="mp3", is_lossy=True, sample_rate=44100, bit_rate=320000)
    bare = Track("/e.flac", codec="flac", sample_rate=44100, bits=16)
    assert [musiclib.quality_detail(t) for t in (hi, cd, fl, mp3, bare)] == [
        "24-bit/96 kHz · 2,304 kbps", "16-bit/44.1 kHz · 1,411 kbps", "32-bit float/192 kHz · 12,288 kbps",
        "MP3 · 320 kbps · 44.1 kHz", "16-bit/44.1 kHz"]
    assert musiclib.quality_detail(Track("/f.wav", codec="pcm_s24le", bits=24, bit_rate=4608000)) == \
        "Hi-Res Lossless · 4,608 kbps"


def test_rust_vectors_hold_pythons_answers():
    """What rust/tools/gen_player_vectors.py wrote for 2.5 is what this module answers
    (the Rust core is compared with the same files in leaderboard_vectors.rs and
    library_vectors.rs)."""
    data = json.loads((TESTS / "python_leaderboard.json").read_text(encoding="utf-8"))
    plays = [Play(**p) for p in data["plays"]]
    assert data["sorts"] == stats.LEADERBOARD_SORTS and data["span_units"] == stats.SPAN_UNITS
    for kind in stats.LEADERBOARD_KINDS:
        for top, rows in data["boards_time"][kind].items():
            assert stats.leaderboard(plays, kind, int(top), "time") == rows, (kind, top)
        assert stats.ranked_songs(data["boards_time"][kind][str(stats.TOP)]) == data["ranked_time"][kind]
    assert data["boards"]["genres"]["100"], "the 2.4 plays ranked by genre"
    cases = data["time_cases"]
    plays = [Play(**p) for p in cases["plays"]]
    for by in stats.LEADERBOARD_SORTS:
        for kind in stats.LEADERBOARD_KINDS:
            assert stats.leaderboard(plays, kind, by=by) == cases["boards"][by][kind], (by, kind)
    assert cases["boards"]["plays"]["songs"] != cases["boards"]["time"]["songs"]
    for c in data["spans"]:
        days = stats.span_days(c["unit"], c["offset"], tuple(c["today"]))
        assert (None if days is None else [list(days[0]), list(days[1])]) == c["days"], c
        assert stats.span_name(c["unit"], c["offset"], tuple(c["today"])) == c["name"], c
    assert [[p, *stats.period_span(p)] for p in stats.PERIODS + ["Some day"]] == data["period_spans"]
    lib = json.loads((TESTS / "python_library.json").read_text(encoding="utf-8"))
    details = [musiclib.quality_detail(Track.from_json(c["track"])) for c in lib["quality"]]
    assert details == [c["detail"] for c in lib["quality"]]
    assert sum("kbps" in d and "bit/" in d for d in details) >= 4, "lossless songs with a bit rate"


def test_spans_and_the_quality_tooltip_in_japanese():
    i18n.set_language("日本語")
    try:
        today = (2026, 11, 25)
        assert [i18n.tr_text(stats.span_name(u, o, today)) for u, o in (
            ("day", 0), ("week", 0), ("week", 1), ("week", 5), ("month", 0))] == [
            "2026年11月25日", "2026年11月23日～29日", "2026年11月30日～12月6日", "2026年12月28日～2027年1月3日",
            "2026年11月"]
        assert i18n.tr("{year}", year=stats.span_name("year", 0, today)) == "2026年"
        assert i18n.tr_text(stats.day_title(*today)) == "2026年11月25日 水曜日", "the calendar's title as before"
        hi = Track("/a.flac", codec="flac", sample_rate=96000, bits=24, bit_rate=2304000)
        fl = Track("/c.wav", codec="pcm_f32le", sample_rate=192000, bits=32, is_float=True, bit_rate=12288000)
        assert i18n.tr_text(musiclib.quality_detail(hi)) == "24ビット/96 kHz · 2,304 kbps"
        assert i18n.tr_text(musiclib.quality_detail(fl)) == "32ビット浮動小数点/192 kHz · 12,288 kbps"
        assert i18n.tr("Times played") == "再生回数" and i18n.tr("Time listened") == "再生時間"
    finally:
        i18n.set_language("English")


def test_first_play_of_the_history(tmp_path):
    lib = musiclib.Library(str(tmp_path / "library.sqlite"))
    assert lib.first_play() is None
    t = Track("/m/a.flac", title="A", duration=100.0)
    lib.add_play(t, 2000.0, 50.0)
    lib.add_play(t, 1000.5, 50.0)
    assert lib.first_play() == 1000.5
    lib.clear_history()
    assert lib.first_play() is None


# ------------------------------------------------------------------ the Tk player ---
PLAYS = """
def log_plays():
    # today: First Light three times (2 minutes in all) and Porch Lamp once for 10 minutes;
    # yesterday: Taxi Lights twice; 40 days ago: Carrier Wave; June of last year: Blue Kettle
    now = time.time()
    day0 = stats.span_range("day", 0, now)[0]
    year = time.localtime(now).tm_year
    last_year = time.mktime((year - 1, 6, 15, 12, 0, 0, 0, 1, -1))
    for title, started, secs in [("First Light", now - 9, 40), ("First Light", now - 8, 40), ("First Light", now - 7, 40),
                                 ("Porch Lamp", now - 6, 600), ("Taxi Lights", day0 - 3600, 60),
                                 ("Taxi Lights", day0 - 3500, 60), ("Carrier Wave", now - 40 * 86400, 120),
                                 ("Blue Kettle", last_year, 200)]:
        assert app.lib.add_play(app.by_path[S[title]], started, secs)
    return now

def texts(c):
    return [c.itemcget(i, "text") for i in c.find_all() if c.type(i) == "text"]

def fill_of(c, text):
    return [c.itemcget(i, "fill") for i, _ in items(c, text)]
"""


@needs_ffmpeg
@pytest.mark.skipif(not _has_miniaudio(), reason="miniaudio not installed")
@pytest.mark.skipif(not _gui_available(), reason="needs Tkinter and a display")
def test_cards_lead_on_and_most_played_sorts_and_filters(tmp_path):
    """Your Stats at 1024 × 768: every card is one click target (hand, lift) that opens
    Most Played with its kind, Details or the busiest day; Most Played: Sort by, Genres,
    the period control with ‹ and ›, the empty period, and the controls wrapping."""
    lib = make_library(tmp_path, seconds=2, mp3=False)
    env = _env(tmp_path, lib)
    body = f"""
LIB = {json.dumps(lib)}
S = LIB["songs"]
""" + HELPERS + PLAYS + """
def card(c, label):  # the box of a card's label (inside the card)
    found = items(c, label.upper())
    assert found, (label, texts(c))
    show(c, found[0][1])
    return items(c, label.upper())[0][1]

def loaded():
    app.engine.pause()
    now = state["now"] = log_plays()
    assert app.lib.first_play() < now - 300 * 86400
    app.stats_period = "All time"
    app.show_page("stats")
    root.update()
    st = app.page("stats")
    c = st.area.c
    # ---- the cards: one click target each, with the hand, lifting under the pointer
    box = card(c, "Your top song")
    x, y = (box[0] + box[2]) / 2 - c.canvasx(0), (box[1] + box[3]) / 2 - c.canvasy(0)
    c.event_generate("<Motion>", x=int(x), y=int(y))
    root.update()
    assert str(c.cget("cursor")) == "hand2"
    hover = st.hover
    assert hover is not None and hover[2] - hover[0] > 150 and hover[3] - hover[1] == 150, hover
    lifted = items(c, "YOUR TOP SONG")[0][1]
    assert lifted[1] == box[1] - st.CARD_LIFT, (box, lifted)
    # the big text is not a link of its own any more: the song is not played, the card leads on
    name = [b for i, b in items(c, "First Light") if c.itemcget(i, "font") == str(st.theme.f("stat_big"))][0]
    press_at(c, name)
    assert app.current is None and app.page_name == "most_played", (app.current, app.page_name)
    assert (app.leaderboard_kind, app.leaderboard_span) == ("songs", ("all", 0))
    for label, kind in (("Your top artist", "artists"), ("Your top album", "albums"), ("Your top genre", "genres")):
        app.go_back()
        root.update()
        assert app.page_name == "stats"
        press_at(c, card(c, label))
        assert (app.page_name, app.leaderboard_kind) == ("most_played", kind), (label, app.page_name)
    mp = app.page("most_played")
    mc = mp.area.c
    assert [r["name"] for r in mp.rows] == ["Ambient", "Jazz", "Indie Pop", "Electronic"], mp.rows
    click_text(mc, "Jazz", lambda b: b[0] > 100)
    assert (app.page_name, app.page("group").kind, app.page("group").name) == ("group", "genres", "Jazz")
    # the other cards: Details (the listening hour at its chart), and the busiest day in the calendar
    app.show_page("stats")
    root.update()
    press_at(c, card(c, "Minutes listened"))
    assert app.page_name == "stats_details"
    dp = app.page("stats_details")
    app.go_back()
    root.update()
    press_at(c, card(c, "Your listening hour"))
    assert app.page_name == "stats_details" and dp.anchors["hour"] > 100, dp.anchors
    wait(lambda: abs(dp.area.c.canvasy(0) - (dp.anchors["hour"] - 8)) <= 24, biggest_day, what="Details at the hours")

def biggest_day():
    st = app.page("stats")
    c = st.area.c
    app.go_back()
    root.update()
    st.show_month(2020, 1)  # another month, and no day chosen
    c.yview_moveto(1.0)
    root.update()
    today = time.localtime(state["now"] - 6)
    press_at(c, card(c, "Your biggest day"))
    assert app.page_name == "stats" and st.day == (today.tm_year, today.tm_mon, today.tm_mday), st.day
    assert st.month == (today.tm_year, today.tm_mon) and c.canvasy(0) == 0, (st.month, c.canvasy(0))
    assert [r["title"] for r in st.day_rows][:1] == ["Porch Lamp"], st.day_rows
    # ---- opened from Your Stats, Most Played starts from that page's period
    for period, span in (("Last 30 days", ("month", 0)), ("This year", ("year", 0)), ("Last year", ("year", -1)),
                         ("All time", ("all", 0))):
        app.stats_period = period
        app.show_page("stats")
        root.update()
        click_text(c, "See All", area=True)
        assert (app.page_name, app.leaderboard_span) == ("most_played", span), (period, app.leaderboard_span)
        if period == "Last year":
            rows = app.page("most_played").rows  # the genres of last year: Blue Kettle's
            assert [r["name"] for r in rows] == ["Jazz"] and rows[0]["paths"] == [S["Blue Kettle"]], rows
    most_played()

def nav(mp, delta):  # click ‹ (-1) or › (+1) of the period control
    mc = mp.area.c
    unit, offset = app.leaderboard_span
    title = player_stats.span_text(unit, offset)
    tw = player_stats._nav_width(mp, unit, title)
    b = items(mc, title)[-1][1]
    cx, cy = (b[0] + b[2]) / 2 + delta * (tw / 2 + 15), (b[1] + b[3]) / 2
    press_at(mc, (cx - 3, cy - 3, cx + 3, cy + 3))

def most_played():
    now = state["now"]
    mp = app.page("most_played")
    mc = mp.area.c
    T = mp.theme
    click_text(mc, "Songs")
    assert app.leaderboard_kind == "songs" and app.leaderboard_by == "plays"
    names = lambda: [r["name"] for r in mp.rows]
    assert names() == ["First Light", "Taxi Lights", "Porch Lamp", "Blue Kettle", "Carrier Wave"], names()
    # ---- Sort by: the number sorted by in the text colour, the other in secondary text
    assert {"Sort by", "Times played", "Time listened", "Genres"} <= set(texts(mc)), texts(mc)
    assert fill_of(mc, "3 plays") == [T.c("text")] and fill_of(mc, "10 min") == [T.c("text2")]
    click_text(mc, "Time listened")
    assert app.leaderboard_by == "time"
    assert names() == ["Porch Lamp", "Blue Kettle", "First Light", "Taxi Lights", "Carrier Wave"], names()
    assert [r["rank"] for r in mp.rows] == [1, 2, 3, 4, 5], mp.rows
    assert fill_of(mc, "3 plays") == [T.c("text2")] and fill_of(mc, "10 min") == [T.c("text")]
    # Play: the ranked songs in this order
    play = [w for w in mp.winfo_children()[0].winfo_children() if isinstance(w, PillButton)]
    press(next(b for b in play if b.text == "Play"))
    assert app.current == S["Porch Lamp"] and app.context_name == "Most Played"
    assert app.queue.context[:3] == [S["Porch Lamp"], S["Blue Kettle"], S["First Light"]]
    app.engine.pause()
    # the section of Your Stats shares the switch
    app.show_page("stats")
    root.update()
    st = app.page("stats")
    assert [r["name"] for r in st.boards["songs"]][:2] == ["Porch Lamp", "Blue Kettle"]
    assert {"Sort by", "Times played", "Time listened"} <= set(texts(st.area.c))
    click_text(st.area.c, "Times played", area=True)
    assert app.leaderboard_by == "plays" and st.boards["songs"][0]["name"] == "First Light"
    app.show_page("most_played")
    root.update()
    assert names()[0] == "First Light"
    # ---- the period control: Day, Week, Month, Year, All time, with ‹ and ›
    assert {"Day", "Week", "Month", "Year", "All time"} <= set(texts(mc))
    click_text(mc, "Day")
    assert app.leaderboard_span == ("day", 0)
    today = stats.span_title("day", 0, now)
    assert mp.sub.cget("text") == today and today in texts(mc), (today, texts(mc))
    assert names() == ["First Light", "Porch Lamp"] and mp.older
    nav(mp, +1)  # › is off at the present
    assert app.leaderboard_span == ("day", 0)
    nav(mp, -1)
    assert app.leaderboard_span == ("day", -1) and names() == ["Taxi Lights"], (app.leaderboard_span, names())
    assert mp.sub.cget("text") == stats.span_title("day", -1, now)
    nav(mp, -1)
    assert app.leaderboard_span == ("day", -2) and names() == []
    assert "Nothing was played in this period." in texts(mc), texts(mc)
    nav(mp, +1)
    nav(mp, +1)
    assert app.leaderboard_span == ("day", 0) and names() == ["First Light", "Porch Lamp"]
    for unit in ("Week", "Month"):
        click_text(mc, unit)
        assert app.leaderboard_span == (unit.lower(), 0) and "First Light" in names()
        assert mp.sub.cget("text") == stats.span_title(unit.lower(), 0, now)
    click_text(mc, "Year")
    nav(mp, -1)
    assert app.leaderboard_span == ("year", -1) and "Blue Kettle" in names() and "First Light" not in names()
    assert mp.sub.cget("text") == str(time.localtime(now).tm_year - 1)
    assert not mp.older  # nothing was played before that year: ‹ is off
    nav(mp, -1)
    assert app.leaderboard_span == ("year", -1)
    # ---- the controls fit the page, on more lines where it is narrow
    def lines():
        W = mc.winfo_width()
        marks = ["Songs", "Sort by", "Day", mp.sub.cget("text")]
        boxes = [items(mc, m)[0][1] for m in marks]
        every = [mc.bbox(i) for i in mc.find_all() if mc.bbox(i) and mc.bbox(i)[1] < boxes[-1][3] + 10]
        assert all(b[0] >= 0 and b[2] <= W for b in every), (W, [b for b in every if b[2] > W])
        return [round((b[1] + b[3]) / 2) for b in boxes]
    ys = lines()
    assert ys[0] < ys[2], ys  # the period control under the switch at 1024 × 768
    root.geometry("700x600")
    wait(lambda: mc.winfo_width() < 520, lambda: narrow(mp, lines), what="a narrow window")

def narrow(mp, lines):
    root.update()
    ys = lines()
    assert ys[0] < ys[1] < ys[2] < ys[3], ys  # one group a line
    assert "Blue Kettle" in texts(mp.area.c)
    click_text(mp.area.c, "All time")
    assert app.leaderboard_span == ("all", 0) and len(mp.rows) == 5
    print("OK")
    app.quit()

wait(lambda: len(app.tracks) == len(S) and not app.library_busy(), loaded, what="library")
root.mainloop()
"""
    run_script(tmp_path, body, env)


@needs_ffmpeg
@pytest.mark.skipif(not _has_miniaudio(), reason="miniaudio not installed")
@pytest.mark.skipif(not _gui_available(), reason="needs Tkinter and a display")
def test_quality_tooltip_on_hover(tmp_path):
    """Hovering the Quality cell shows the resolution and the bit rate on All Songs, an
    album's songs, an artist's songs (the album opened from the artist's page), a genre's
    songs and a folder's contents; so do the playback bar's quality badge and the Format
    row of Now Playing's Details."""
    mp3 = _has_mp3_encoder()
    lib = make_library(tmp_path, seconds=2, mp3=mp3)
    env = _env(tmp_path, lib)
    body = f"""
LIB = {json.dumps(lib)}
S = LIB["songs"]
MP3 = {mp3!r}
""" + HELPERS + """
import re
from realhires import musiclib
LOSSLESS = re.compile(r"^(16|24)-bit/(44\\.1|48|96) kHz · [\\d,]+ kbps$")
CASES = []

def hover(table, title, then):  # rest the pointer on the Quality cell of the song `title`
    root.update()
    i = next(k for k, r in enumerate(table.rows) if getattr(r, "title", "") == title)
    col = next((x, w) for c, x, w in table._layout(table.canvas.winfo_width()) if c.key == "quality")
    y = (table.HEADER if table.has_header else 0) + int(i * table.rh - table.top) + table.rh // 2
    assert 0 < y < table.canvas.winfo_height(), (title, y)
    table.hide_tip()
    table.canvas.event_generate("<Motion>", x=10, y=y)  # another cell first: no tooltip there
    root.update()
    assert table._tip_cell is None
    table.canvas.event_generate("<Motion>", x=col[0] + col[1] // 2, y=y)
    def shown():
        tip = table.tip_text
        assert table._tip_win is not None and tip == musiclib.quality_detail(table.rows[i]), tip
        CASES.append(tip)
        table.canvas.event_generate("<Leave>")
        root.update()
        assert table.tip_text == "" and table._tip_win is None
        then(tip)
    wait(lambda: table.tip_text != "", shown, what="the tooltip of " + title)

def loaded():
    app.engine.pause()
    app.show_page("songs")
    def all_songs(tip):
        assert tip == "24-bit/96 kHz · " + tip.split(" · ")[1] and LOSSLESS.match(tip), tip
        album = next(a for a in app.albums if a["title"] == "City Rain")
        app.open_album(album)
        hover(app.page("album").table, "Taxi Lights", album_songs)
    def album_songs(tip):
        assert tip.startswith("16-bit/44.1 kHz · ") and LOSSLESS.match(tip), tip
        # an artist's page shows the artist's albums: a click on one lists its songs
        app.show_page("group", kind="artists", group="June Harbor")
        root.update()
        grid = app.page("group").grid_w
        assert [a["title"] for a in grid.albums] == ["Small Hours"] and grid.winfo_ismapped()
        x, y = grid._pos(0)
        grid.canvas.event_generate("<Button-1>", x=int(x + 40 - grid.canvas.canvasx(0)),
                                   y=int(y + 40 - grid.canvas.canvasy(0)))
        root.update()
        assert app.page_name == "album" and app.page("album").album["title"] == "Small Hours"
        hover(app.page("album").table, "Porch Lamp", artist_songs)
    def artist_songs(tip):
        assert tip.startswith("24-bit/48 kHz · ") and LOSSLESS.match(tip), tip
        app.show_page("group", kind="genres", group="Ambient")  # a genre's songs
        hover(app.page("group").table, "Glacier Song", genre_songs)
    def genre_songs(tip):
        assert tip.startswith("24-bit/96 kHz · ") and LOSSLESS.match(tip), tip
        app.show_page("folders", folder=os.path.dirname(S["Low Pass"]))
        hover(app.page("folders").table, "Low Pass", folder_songs)
    def folder_songs(tip):
        if MP3:
            assert re.match(r"^MP3 · \\d+ kbps · 44\\.1 kHz$", tip), tip
        else:
            assert tip.startswith("16-bit/44.1 kHz · ") and LOSSLESS.match(tip), tip
        badge()
    hover(app.page("songs").table, "First Light", all_songs)

def badge():
    # ---- the playback bar's quality badge
    app.play_list([app.by_path[S["First Light"]]], 0, "All Songs")
    root.update()
    app.engine.pause()
    lcd = app.bar.lcd
    assert lcd.badge_tip.tip_text == ""
    b = lcd.geo["badge"]
    lcd.event_generate("<Motion>", x=int((b[0] + b[2]) / 2), y=int((b[1] + b[3]) / 2))
    def shown():
        tip = lcd.badge_tip.tip_text.splitlines()  # the quality, and what a click opens
        assert lcd.badge_tip.win is not None and LOSSLESS.match(tip[0]) and tip[1:] == ["Playback info"], tip
        assert tip[0] == musiclib.quality_detail(app.by_path[S["First Light"]])
        lcd.event_generate("<Motion>", x=int(b[0]) - 60, y=int((b[1] + b[3]) / 2))  # off the badge
        root.update()
        assert lcd.badge_tip.tip_text == "" and lcd.badge_tip.win is None
        details()
    wait(lambda: lcd.badge_tip.tip_text != "", shown, what="the badge's tooltip")

def details():
    # ---- Now Playing's Details: the Format row
    from realhires import player_quality
    app.show_page("nowplaying")
    app.set_np_tab("details")
    root.update()
    np = app.page("nowplaying")
    view = np.details
    def ready():
        root.update()
        box = player_quality.details_row(view, "Format")
        other = player_quality.details_row(view, "Title")
        assert box and other and box[1] >= other[3] - 2, (box, other)
        view.event_generate("<Motion>", x=int((other[0] + other[2]) / 2 - view.canvasx(0)),
                            y=int((other[1] + other[3]) / 2 - view.canvasy(0)))
        root.update()
        assert not np.details_tip.inside
        view.event_generate("<Motion>", x=int((box[0] + box[2]) / 2 - view.canvasx(0)),
                            y=int((box[1] + box[3]) / 2 - view.canvasy(0)))
        def shown():
            assert LOSSLESS.match(np.details_tip.tip_text), np.details_tip.tip_text
            print(CASES)
            print("OK")
            app.quit()
        wait(lambda: np.details_tip.tip_text != "", shown, what="the Format row's tooltip")
    wait(lambda: player_quality.details_row(view, "Format") is not None, ready, what="Details")

wait(lambda: len(app.tracks) == len(S) and not app.library_busy(), loaded, what="library")
root.mainloop()
"""
    run_script(tmp_path, body, env)


@needs_ffmpeg
@pytest.mark.skipif(not _has_miniaudio(), reason="miniaudio not installed")
@pytest.mark.skipif(not _gui_available(), reason="needs Tkinter and a display")
def test_most_played_and_quality_tooltip_in_japanese(tmp_path):
    """Most Played's controls and period names in Japanese, and the tooltip's text."""
    lib = make_library(tmp_path, seconds=2, mp3=False)
    env = _env(tmp_path, lib, language="日本語")
    body = f"""
LIB = {json.dumps(lib)}
S = LIB["songs"]
""" + HELPERS + PLAYS + """
import re

def loaded():
    app.engine.pause()
    log_plays()
    app.stats_period = "This year"
    player_stats.open_most_played(app, "genres")
    root.update()
    mp = app.page("most_played")
    mc = mp.area.c
    assert {"曲", "アーティスト", "アルバム", "ジャンル", "並べ替え", "再生回数", "再生時間", "日", "週", "月", "年",
            "これまでのすべて"} <= set(texts(mc)), texts(mc)
    assert re.match(r"^\\d{4}年$", mp.sub.cget("text")) and mp.sub.cget("text") in texts(mc), mp.sub.cget("text")
    for unit, shape in (("day", r"^\\d{4}年\\d{1,2}月\\d{1,2}日$"), ("month", r"^\\d{4}年\\d{1,2}月$"),
                        ("week", r"^\\d{4}年\\d{1,2}月\\d{1,2}日～(\\d{4}年)?(\\d{1,2}月)?\\d{1,2}日$")):
        app.leaderboard_span = (unit, 0)
        mp.refresh()
        root.update()
        title = mp.sub.cget("text")
        assert re.match(shape, title) and title in texts(mc), (unit, title)
        W = mc.winfo_width()
        assert all(mc.bbox(i)[2] <= W for i in mc.find_all() if mc.bbox(i)), "the controls fit in Japanese"
    app.leaderboard_span = ("day", -3)
    mp.refresh()
    root.update()
    assert "この期間には何も再生されていません。" in texts(mc), texts(mc)
    # the Quality cell's tooltip
    app.show_page("songs")
    root.update()
    t = app.page("songs").table
    i = next(k for k, r in enumerate(t.rows) if r.title == "First Light")
    tip = t.show_tip(i, "quality")
    assert re.match(r"^24ビット/96 kHz · [\\d,]+ kbps$", tip), tip
    t.hide_tip()
    print("OK")
    app.quit()

wait(lambda: len(app.tracks) == len(S) and not app.library_busy(), loaded, what="library")
root.mainloop()
"""
    run_script(tmp_path, body, env)
