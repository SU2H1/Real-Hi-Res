"""2.4 (docs/player-spec.md, "2.4" items 1 and 2): the Folders page scrolls as one page,
the names on Your Stats are links, and the leaderboard ("Most Played").  The core's
leaderboard is checked here and against the Rust vectors; the Tk pages run in a
subprocess, as test_player_gui.py's do (skipped without Tk or a display; locally:
xvfb-run -a python -m pytest)."""

import json
import os
import shutil
from pathlib import Path

import pytest

from player_library import make_library
from test_player_gui import _gui_available, _has_miniaudio, run_script
from realhires import stats
from realhires.stats import Play

needs_ffmpeg = pytest.mark.skipif(shutil.which("ffmpeg") is None, reason="ffmpeg not installed")
VECTORS = Path(__file__).resolve().parents[1] / "rust" / "crates" / "core" / "tests" / "python_leaderboard.json"


def _play(path, started, seconds, title="", artist="", album="", album_artist="", duration=200.0):
    return Play(path, started, seconds, duration, title, artist, album, album_artist)


def test_leaderboard_ranks_by_plays_then_minutes_and_ties_share_a_rank():
    plays = [
        _play("/m/a.flac", 1, 60, "Alpha", "Ann", "One"), _play("/m/a.flac", 2, 60, "Alpha", "Ann", "One"),
        _play("/m/b.flac", 3, 120, "Bravo", "Ann", "One"),  # 1 play, 2 min
        _play("/m/c.flac", 4, 119.6, "charlie", "Bob", "Two"),  # 1 play, 2 min (rounded): shares the rank
        _play("/m/d.flac", 5, 300, "Delta", "Bob", "Two"),  # 1 play, 5 min: before the 2-minute ones
        _play("/m/e.flac", 6, 10, "Echo", "Cy", "Three"),  # too short: not a play
        _play("/m/e.flac", 7, 12, "Echo", "Cy", "Three"),
        _play("/m/f.flac", 8, 45, " ", " ", "", album_artist="Dee"),  # no title: the file's name
    ]
    songs = stats.leaderboard(plays, "songs")
    assert [(r["rank"], r["name"], r["plays"], r["minutes"]) for r in songs] == [
        (1, "Alpha", 2, 2), (2, "Delta", 1, 5), (3, "Bravo", 1, 2), (3, "charlie", 1, 2), (5, "f", 1, 1)]
    assert songs[0]["key"] == songs[0]["paths"][0] == "/m/a.flac" and songs[0]["artist"] == "Ann"
    assert songs[4]["artist"] == "Dee" and songs[4]["seconds"] == 45
    artists = stats.leaderboard(plays, "artists")
    assert [(r["rank"], r["name"], r["plays"], r["minutes"], r["artist"]) for r in artists] == [
        (1, "Ann", 3, 4, ""), (2, "Bob", 2, 7, ""), (3, "Dee", 1, 1, "")]
    assert artists[0]["paths"] == ["/m/a.flac", "/m/b.flac"]  # the most played first
    assert artists[1]["paths"] == ["/m/d.flac", "/m/c.flac"]  # one play each: the longer first
    albums = stats.leaderboard(plays, "albums")
    assert [(r["rank"], r["name"], r["artist"]) for r in albums] == [
        (1, "One", "Ann"), (2, "Two", "Bob"), (3, "Unknown Album", "Dee")]
    assert albums[0]["key"] == "one\x00ann"
    # up to `top` rows; Play and Shuffle take the songs in rank order, each once
    assert len(stats.leaderboard(plays, "songs", top=2)) == 2 and stats.leaderboard(plays, "songs", top=0) == []
    assert stats.ranked_songs(artists) == ["/m/a.flac", "/m/b.flac", "/m/d.flac", "/m/c.flac", "/m/f.flac"]
    assert stats.ranked_songs(songs + songs) == [r["key"] for r in songs]
    assert stats.leaderboard([], "albums") == []
    with pytest.raises(ValueError):
        stats.leaderboard(plays, "genres")


def test_leaderboard_keeps_a_hundred_rows():
    # two songs each with the same minutes: they share a rank, by name
    plays = [_play(f"/m/{i:03d}.flac", i, 31 + 60 * (i // 2), f"Song {i:03d}", "Ann", "One") for i in range(130)]
    rows = stats.leaderboard(plays, "songs")
    assert len(rows) == stats.TOP == 100
    assert [(r["rank"], r["name"]) for r in rows[:3]] == [(1, "Song 128"), (1, "Song 129"), (3, "Song 126")]
    assert rows[-1]["rank"] == 99


def test_rust_vectors_hold_pythons_answers():
    """python_leaderboard.json is what rust/tools/gen_player_vectors.py writes from this
    module; the Rust core is compared with it in leaderboard_vectors.rs."""
    data = json.loads(VECTORS.read_text(encoding="utf-8"))
    plays = [Play(**p) for p in data["plays"]]
    assert data["kinds"] == stats.LEADERBOARD_KINDS
    for kind in stats.LEADERBOARD_KINDS:
        for top, rows in data["boards"][kind].items():
            assert stats.leaderboard(plays, kind, int(top)) == rows, (kind, top)
        assert stats.ranked_songs(data["boards"][kind][str(stats.TOP)]) == data["ranked"][kind]
    ranks = [r["rank"] for k in stats.LEADERBOARD_KINDS for r in data["boards"][k]["100"]]
    assert len(ranks) != len(set(ranks)), "the vectors hold ties"
