"""The 2.0 additions to the player's core: the library's groupings (album
artists, years, composers, folders), the play history and "Your Stats",
playlists, dislikes, named EQ presets with their files, and themes."""

import os
import time

import pytest

from realhires import eq, musiclib, playerstate, stats, themes
from realhires.musiclib import Track
from realhires.playqueue import PlayQueue


def P(path: str) -> str:
    """A path as the library holds it on this system ("/m/A" is "D:\\m\\A" on Windows)."""
    return os.path.abspath(path)


def _t(path, **kw):
    return Track(path=P(path), **kw)


LIB = [
    _t("/m/A/01.flac", title="One", artist="Ann", album="Alpha", album_artist="Ann", genre="Pop", year="2001",
       composer="Zed", track=1, duration=200.0),
    _t("/m/A/02.flac", title="Two", artist="Ann feat. Bo", album="Alpha", album_artist="Ann", genre="pop",
       year="2001", track=2, duration=180.0),
    _t("/m/B/sub/03.flac", title="Three", artist="Cy", album="Beta", genre="Jazz", year="1999", composer="zed",
       track=1, duration=20.0),
    _t("/m/B/04.mp3", title="Four", artist="Cy", album="", year="x", duration=240.0, is_lossy=True, codec="mp3"),
]


def test_groupings():
    # 2.1: "Ann feat. Bo" is two artists, and the song is on both pages
    assert [g["name"] for g in musiclib.artists(LIB)] == ["Ann", "Bo", "Cy"]
    assert [(g["name"], len(g["tracks"])) for g in musiclib.album_artists(LIB)] == [("Ann", 2), ("Cy", 2)]
    assert [g["name"] for g in musiclib.genres(LIB)] == ["Jazz", "Pop", "Unknown Genre"]
    assert [(g["name"], len(g["tracks"])) for g in musiclib.composers(LIB)] == [("Unknown Composer", 2), ("Zed", 2)]
    assert [g["name"] for g in musiclib.years(LIB)] == ["2001", "1999", "Unknown Year"]
    folders, songs = musiclib.folder_view(LIB, P("/m"))
    assert folders == [(P("/m/A"), "A", 2), (P("/m/B"), "B", 2)] and songs == []
    folders, songs = musiclib.folder_view(LIB, P("/m/B"))
    assert folders == [(P("/m/B/sub"), "sub", 1)] and [t.title for t in songs] == ["Four"]


def test_play_history_and_stats(tmp_path):
    lib = musiclib.Library(str(tmp_path / "library.sqlite"))
    t0 = 1_700_000_000.0  # 2023-11-14 22:13:20 UTC
    assert not lib.add_play(LIB[0], t0, 0.5)          # too short to log
    lib.add_play(LIB[0], t0, 200.0)
    lib.add_play(LIB[0], t0 + 3600, 40.0)
    lib.add_play(LIB[2], t0 + 86400, 10.0)              # half of a 20 s song: counts
    lib.add_play(LIB[1], t0 + 2 * 86400, 5.0)           # skipped: logged, not a play
    plays = lib.plays()
    assert [p.path for p in plays] == [P("/m/A/01.flac"), P("/m/A/01.flac"), P("/m/B/sub/03.flac"),
                                       P("/m/A/02.flac")]
    assert lib.recently_played() == [P("/m/A/02.flac"), P("/m/B/sub/03.flac"), P("/m/A/01.flac")]
    assert len(lib.plays(since=t0 + 1000)) == 3

    utc = time.gmtime
    st = stats.compute(plays, localtime=utc)
    assert (st["plays"], st["logged"], st["songs"], st["artists"]) == (3, 4, 3, 3)
    assert st["seconds"] == 255.0
    assert st["top_songs"][0]["key"] == P("/m/A/01.flac") and st["top_songs"][0]["plays"] == 2
    assert st["top_artists"][0]["name"] == "Ann" and st["top_artists"][0]["songs"] == 1
    assert st["by_hour"][22] == 215.0 and st["by_hour"][23] == 40.0
    assert st["days"] == 3 and st["streak"] == 3 and st["busiest_day"] == ["2023-11-14", 240.0]
    cards = stats.cards(st)
    labels = [c[0] for c in cards]
    assert labels[:3] == ["Minutes listened", "Your top song", "Your top artist"]
    assert cards[0][1] == "4" and cards[1][2] == "Ann · 2 plays"
    assert ("Your listening hour", "10 PM", "when you listen most") in cards
    assert ("Longest streak", "3 days", "in a row with music") in cards
    assert stats.cards(stats.compute([], localtime=utc)) == []
    start, end = stats.period_range("Last year", t0, localtime=utc,
                                    mktime=lambda tt: __import__("calendar").timegm(tt))
    assert (start, end) == (1640995200.0, 1672531200.0)
    lib.close()

    ls = stats.library_stats(LIB)
    assert (ls["songs"], ls["albums"], ls["seconds"]) == (4, 3, 640.0)
    assert ls["qualities"] == [["Hi-Res Lossless", 0], ["Lossless", 3], ["Lossy", 1]]


def test_stats_calendar(tmp_path):
    """The month calendar of Your Stats: the grid, the minutes and tints of each
    day, a day's plays newest first, and the local midnights the plays are read
    between (local time is injected: Tokyo, UTC+9)."""
    import calendar

    def tokyo(ts):
        return time.gmtime(ts + 9 * 3600)

    def tokyo_mktime(tt):
        return calendar.timegm(tt) - 9 * 3600

    assert stats.month_grid(2026, 11)[0] == [0, 0, 0, 0, 0, 0, 1]
    assert stats.month_grid(2026, 11)[-1] == [30, 0, 0, 0, 0, 0, 0] and len(stats.month_grid(2026, 11)) == 6
    assert stats.month_grid(2021, 2) == [list(range(w, w + 7)) for w in (1, 8, 15, 22)]
    assert stats.month_title(2026, 11) == "November 2026" and stats.day_title(2026, 11, 24) == "Tuesday 24 November 2026"
    assert stats.shift_month(2026, 12, 1) == (2027, 1) and stats.shift_month(2026, 1, -13) == (2024, 12)
    assert stats.month_bounds(2026, 12, mktime=tokyo_mktime) == (1796050800.0, 1798729200.0)
    start, end = stats.day_bounds(2024, 2, 28, mktime=tokyo_mktime)
    assert end - start == 86400.0 and tokyo(start)[:6] == (2024, 2, 28, 0, 0, 0)
    assert stats.day_bounds(2024, 2, 29, mktime=tokyo_mktime)[1] == stats.month_bounds(2024, 3, mktime=tokyo_mktime)[0]

    lib = musiclib.Library(str(tmp_path / "library.sqlite"))
    midnight = stats.day_bounds(2026, 11, 24, mktime=tokyo_mktime)[0]
    lib.add_play(LIB[0], midnight - 60, 120.0)           # 23:59 the day before
    lib.add_play(LIB[1], midnight + 14 * 3600 + 300, 185.0)  # 14:05
    lib.add_play(LIB[2], midnight + 20 * 3600, 20.0)      # 20:00, a third of a minute
    lib.add_play(LIB[3], midnight + 2 * 86400, 600.0)     # 26 November
    month = lib.plays(*stats.month_bounds(2026, 11, mktime=tokyo_mktime))
    minutes = stats.month_minutes(month, 2026, 11, localtime=tokyo)
    assert len(minutes) == 30 and minutes[22] == 2.0 and minutes[23] == pytest.approx(205.0 / 60) and minutes[25] == 10.0
    assert sum(1 for m in minutes if m) == 3
    levels = stats.month_levels(minutes)
    assert (levels[22], levels[23], levels[25], levels[0]) == (1, 2, 4, 0)
    assert stats.month_levels([0.0] * 30) == [0] * 30
    day = lib.plays(*stats.day_bounds(2026, 11, 24, mktime=tokyo_mktime))
    assert [p.path for p in day] == [LIB[1].path, LIB[2].path]
    rows = stats.day_plays(day, 2026, 11, 24, localtime=tokyo)
    assert [(r["time"], r["title"], r["artist"], r["minutes"]) for r in rows] == [
        ("20:00", "Three", "Cy", "<1 min"), ("14:05", "Two", "Ann feat. Bo", "3 min")]
    assert stats.day_plays(month, 2026, 11, 24, localtime=tokyo) == rows  # other days are left out
    assert stats.day_summary(rows) == "2 songs played, 3 minutes"
    assert stats.day_summary([]) == "No music on this day."
    assert stats.minutes_label(74_043.0) == "1,234 min"
    lib.close()


def test_playlists_dislikes_and_eq_presets(tmp_path):
    st = playerstate.PlayerState()
    pid = st.create_playlist("Road Trip", ["/m/a", "/m/b", "/m/a"], now=1000.0)
    assert st.playlist(pid)["songs"] == ["/m/a", "/m/b"]
    pid2 = st.create_playlist("road trip", now=1000.0)
    assert st.playlist(pid2)["name"] == "road trip 2" and pid2 != pid
    assert st.add_to_playlist(pid, ["/m/b", "/m/c"]) == 1
    st.move_in_playlist(pid, "/m/c", -2)
    assert st.playlist(pid)["songs"] == ["/m/c", "/m/a", "/m/b"]
    st.remove_from_playlist(pid, ["/m/a"])
    st.rename_playlist(pid2, "Night")
    st.delete_playlist(pid2)
    st.set_liked("/m/a", True)
    st.set_disliked("/m/a", True)
    assert not st.is_liked("/m/a") and st.is_disliked("/m/a")
    st.set_liked("/m/a", True)
    assert st.is_liked("/m/a") and not st.is_disliked("/m/a")
    st.set_disliked("/m/z", True)
    with pytest.raises(playerstate.InvalidName):
        st.save_eq_preset("rock", eq.default_state())
    st.eq = eq.apply_preset(eq.default_state(), "Jazz")
    st.save_eq_preset("  My   Jazz ", st.eq)
    assert st.matching_eq_preset() == "My Jazz"
    with pytest.raises(playerstate.InvalidName):
        st.save_eq_preset("my jazz", st.eq)
    st.rename_eq_preset("My Jazz", "Late Jazz")
    path = str(tmp_path / "player.json")
    playerstate.save(st, path)
    again = playerstate.load(path)
    assert again.playlists == st.playlists and again.disliked == ["/m/z"]
    assert [p["name"] for p in again.eq_presets] == ["Late Jazz"] and again.matching_eq_preset() == "Late Jazz"
    again.delete_eq_preset("Late Jazz")
    assert again.eq_presets == []


def test_shuffle_leaves_out_disliked_songs():
    songs = [t.song() for t in LIB]
    paths = [t.path for t in LIB]
    q = PlayQueue(songs, paths, shuffle="songs", seed=7, disliked={P("/m/A/02.flac"), P("/m/B/04.mp3")})
    q.start(P("/m/A/01.flac"))
    assert q.order == [P("/m/A/01.flac"), P("/m/B/sub/03.flac")]
    q = PlayQueue(songs, paths, shuffle="songs", seed=7, disliked={P("/m/A/01.flac")})
    assert q.start(P("/m/A/01.flac")) == P("/m/A/01.flac") and P("/m/A/01.flac") not in q.order[1:]
    q = PlayQueue(songs, paths, shuffle="off", disliked={P("/m/A/02.flac")})
    q.start(P("/m/A/01.flac"))
    assert q.order == paths  # in order: nothing is left out


def test_eq_preset_files():
    s = eq.apply_preset(eq.default_state(), "Rock")
    s["parametric"] = {"enabled": True, "preamp": -1.5,
                       "bands": [{"type": "high_shelf", "freq": 8000, "gain": -3, "q": 0.71, "on": True},
                                 {"type": "notch", "freq": 3000, "gain": 0, "q": 5, "on": False}]}
    name, back, notes = eq.import_eq(eq.export_json("Mine", s), "File")
    assert name == "Mine" and back == eq.normalize(s) and notes == []
    apo = eq.export_apo("Mine", s)
    assert "Filter 11: ON HSC Fc 8000.0 Hz Gain -3.0 dB Q 0.71" in apo
    assert "Filter 12: OFF NO Fc 3000.0 Hz Q 5.00" in apo
    auto = ("Preamp: -6.2 dB\nFilter 1: ON PK Fc 105 Hz Gain 3.5 dB Q 0.70\n"
            "Filter 2: ON LSC Fc 105 Hz Gain 4.1 dB Q 0.71\nFilter 3: ON BP Fc 1 Hz\n")
    name, st, notes = eq.import_eq(auto, "HD 600")
    assert name == "HD 600" and st["enabled"] and not st["graphic"]["enabled"]
    assert st["parametric"]["preamp"] == -6.2 and len(st["parametric"]["bands"]) == 2
    assert st["parametric"]["bands"][1] == {"type": "low_shelf", "freq": 105.0, "gain": 4.1, "q": 0.71, "on": True}
    assert notes == ["Filters of the type BP are not supported and were left out."]
    _, st, _ = eq.import_eq("GraphicEQ: 20 -1; 1000 0; 20000 -6", "G")
    assert st["graphic"]["enabled"] and st["graphic"]["gains"][5] == 0.0 and st["graphic"]["gains"][0] < 0
    with pytest.raises(eq.EqFileError):
        eq.import_eq("hello", "x")


def test_themes(tmp_path, monkeypatch):
    monkeypatch.setenv(themes.THEMES_ENV, str(tmp_path / "themes.json"))
    assert themes.load_custom() == []
    assert themes.names([])[:3] == ["Match the system", "Light", "Dark"]
    assert themes.resolve("Match the system", True, [])["name"] == "Dark"
    assert themes.resolve("Nope", False, [])["name"] == "Light"
    mine = dict(themes.BUILTIN["Midnight"], name=themes.copy_name("Midnight", []))
    mine["colors"] = dict(mine["colors"], accent="#00FF00", text="nope")
    mine = themes.normalize(mine)
    assert mine["name"] == "Midnight copy" and mine["colors"]["accent"] == "#00ff00"
    assert mine["colors"]["text"] == themes.BUILTIN["Dark"]["colors"]["text"]
    themes.save_custom([mine])
    loaded = themes.load_custom()
    assert loaded == [mine] and themes.resolve("Midnight copy", False, loaded) == mine
    with pytest.raises(themes.ThemeError):
        themes.check_name("dark", loaded)
    again = themes.import_theme(themes.export_theme(mine), loaded)
    assert again["name"] == "Midnight copy 2" and again["colors"] == mine["colors"]
    with pytest.raises(themes.ThemeError):
        themes.import_theme("{}", loaded)
    assert os.path.exists(tmp_path / "themes.json")


def test_several_artists_in_one_field():
    """2.1, the user's example: "Mariah Carey, Michael Bublé" is two artists when either
    has songs of their own; band names with a comma or '&' stay whole."""
    T = musiclib.Track
    lib = [T("/a", title="Duet", artist="Mariah Carey, Michael Bublé"), T("/b", title="Solo", artist="Michael Bublé"),
           T("/c", title="Band", artist="Earth, Wind & Fire"), T("/d", title="Duo", artist="Simon & Garfunkel"),
           T("/e", title="Feat", artist="Drake (feat. Rihanna)"), T("/f", title="Tags", artist="One;Two"),
           T("/g", title="Many", artist="Michael Bublé & Someone x Other / More", album_artist="Michael Bublé")]
    known = musiclib.known_artists(lib)
    assert musiclib.split_artists("Mariah Carey, Michael Bublé", known) == ["Mariah Carey", "Michael Bublé"]
    assert musiclib.split_artists("Earth, Wind & Fire", known) == ["Earth, Wind & Fire"]
    assert musiclib.split_artists("Simon & Garfunkel", known) == ["Simon & Garfunkel"]
    assert musiclib.split_artists("Drake (feat. Rihanna)", known) == ["Drake", "Rihanna"]
    assert musiclib.split_artists("One;Two", known) == ["One", "Two"]
    assert musiclib.split_artists("Michael Bublé & Someone x Other / More", known) == [
        "Michael Bublé", "Someone", "Other", "More"]
    assert musiclib.split_artists("A FEAT. b, B", set()) == ["A", "b, B"]
    groups = {g["name"]: [t.path for t in g["tracks"]] for g in musiclib.artists(lib)}
    assert groups["Michael Bublé"] == ["/a", "/g", "/b"] and groups["Mariah Carey"] == ["/a"]
    assert "Earth, Wind & Fire" in groups and "Mariah Carey, Michael Bublé" not in groups
    assert [g["name"] for g in musiclib.album_artists(lib)][:3] == ["Drake", "Earth, Wind & Fire", "Mariah Carey"]


def test_a_preset_saved_with_the_equalizer_off_can_be_found_and_deleted(tmp_path):
    """The user's report "my preset delete button doesn't work": the preset match left
    no preset to delete once the EQ was switched on (choosing a preset keeps it on)."""
    st = playerstate.PlayerState()
    st.eq = eq.apply_preset(eq.default_state(), "Rock")
    st.eq["enabled"] = False
    st.save_eq_preset("Quiet", st.eq)
    st.eq = dict(st.eq, enabled=True)
    assert st.matching_eq_preset() == "Quiet"
    st.eq["graphic"] = dict(st.eq["graphic"], gains=[g + 0.5 for g in st.eq["graphic"]["gains"]])
    assert st.matching_eq_preset() == ""


def test_song_list_columns_round_trip(tmp_path):
    st = playerstate.PlayerState()
    assert st.columns_of("songs") == ["title", "artist", "album", "genre", "time", "quality", "like"]
    assert st.columns_of("album")[:2] == ["num", "title"]
    assert st.toggle_column("songs", "title") == st.columns_of("songs")  # Title cannot be hidden
    st.toggle_column("songs", "year")
    st.toggle_column("songs", "like")
    st.toggle_column("folders", "bit_rate")
    path = str(tmp_path / "player.json")
    playerstate.save(st, path)
    again = playerstate.load(path)
    assert again.columns_of("songs") == ["title", "artist", "album", "genre", "year", "time", "quality"]
    assert "bit_rate" in again.columns_of("folders") and again.columns_of("album") == st.columns_of("album")
    again.reset_columns("songs")
    assert again.columns_of("songs") == list(playerstate.DEFAULT_COLUMNS["songs"])
    odd = playerstate.PlayerState.from_json({"columns": {"songs": ["nope", "file"], "album": [], "x": ["title"]}})
    assert odd.columns_of("songs") == ["title", "file"] and odd.columns_of("album") == st.columns_of("album")


def test_quality_tooltip_and_sort():
    hi = Track("/a.flac", codec="flac", sample_rate=96000, bits=24)
    cd = Track("/b.flac", codec="flac", sample_rate=44100, bits=16)
    fl = Track("/c.wav", codec="pcm_f32le", sample_rate=192000, bits=32, is_float=True)
    mp3 = Track("/d.mp3", codec="mp3", is_lossy=True, sample_rate=44100, bit_rate=320000)
    assert [musiclib.quality(t) for t in (hi, cd, mp3)] == ["Hi-Res Lossless", "Lossless", "MP3 320 kbps"]
    assert [musiclib.quality_detail(t) for t in (hi, cd, fl, mp3)] == [
        "24-bit/96 kHz", "16-bit/44.1 kHz", "32-bit float/192 kHz", "MP3 · 320 kbps · 44.1 kHz"]
    assert sorted([fl, hi, mp3, cd], key=musiclib.quality_sort) == [mp3, cd, hi, fl]


def test_renaming_a_group_keeps_the_rest_of_the_date():
    from realhires import tagedit

    assert tagedit.renamed_date("2001-05-03", "2004") == "2004-05-03"
    assert tagedit.renamed_date("", "2004") == tagedit.renamed_date("May 2001", "2004") == "2004"
    assert tagedit.group_changes("years", "2004", {"date": "2001-05"}) == {"date": "2004-05"}
    assert tagedit.group_changes("genres", "Jazz", {}) == {"genre": "Jazz"}
    with pytest.raises(tagedit.TagError, match="four digits"):
        tagedit.check_group_name("years", "04")
    assert tagedit.check_group_name("genres", "  Classic   Rock ") == "Classic Rock"
