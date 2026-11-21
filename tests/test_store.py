"""Saved results (realhires/store.py) and their use by the CLI."""

import math
import os
import shutil

import pytest
from conftest import ffmpeg_write, music_like

from realhires import cli, settings, store
from realhires.store import ResultStore, settings_key
from realhires.verdict import ERROR, FileResult, analyze_file

needs_ffmpeg = pytest.mark.skipif(shutil.which("ffmpeg") is None, reason="ffmpeg not installed")


@pytest.fixture
def song(tmp_path):
    return ffmpeg_write(tmp_path / "song.flac", music_like(96000, 6, top_hz=30000, seed=11), 96000,
                        ["-c:a", "flac", "-sample_fmt", "s32"])


@needs_ffmpeg
def test_result_and_graphs_round_trip_exactly(song, tmp_path):
    r = analyze_file(song, keep_plot=True)
    r.notes.append("ünïcode ✓")
    r.cliffs_db[44100] = -math.inf
    key = settings_key()
    with ResultStore() as st:
        assert st.put(r, key, None, plot=r.plot)
        got = st.lookup(song, key)
        plot = st.get_plot(song, key)
    a, b = r.to_dict(), got.to_dict()
    assert a.keys() == b.keys()
    assert all(a[k] == b[k] or (a[k] != a[k] and b[k] != b[k]) for k in a)  # NaN == NaN here
    assert got.cliffs_db[44100] == -math.inf and isinstance(next(iter(got.bit_usage)), int)
    assert got.saved["version"] and got.saved["graphs"] and not got.saved["outdated"]
    assert plot["spectrogram"]["data"] == r.plot["spectrogram"]["data"]
    assert plot["spectrogram"]["shape"] == r.plot["spectrogram"]["shape"] and plot["db"] == r.plot["db"]


@needs_ffmpeg
def test_reuse_rules(song, tmp_path, monkeypatch):
    key = settings_key()
    with ResultStore() as st:
        assert st.put(analyze_file(song), key)
        assert st.lookup(song, key) is not None
        assert st.lookup(song, key, wanted=30) is not None          # a whole-file result answers a quick scan
        quick = analyze_file(song, max_seconds=2)
        assert st.put(quick, key, wanted=2)
        assert st.lookup(song, key) is None                        # ...but not the other way round
        assert st.lookup(song, key, wanted=2) is not None
        assert st.put(analyze_file(song), key)

        cfg = settings.Settings()
        cfg.colormap = "gray"                                      # display settings do not matter
        assert settings_key(cfg) == key
        cfg.cliff_db = 30.0                                        # detection settings do
        assert st.lookup(song, settings_key(cfg)) is None

        moved = os.path.join(tmp_path, "sub", "renamed.flac")
        os.makedirs(os.path.dirname(moved))
        os.rename(song, moved)                                     # keeps the modification time
        r = st.lookup(moved, key)
        assert r is not None and r.saved["moved_from"] == os.path.abspath(song) and r.path == moved

        other = ffmpeg_write(tmp_path / "other.flac", music_like(96000, 6, top_hz=30000, seed=12), 96000,
                             ["-c:a", "flac", "-sample_fmt", "s32"])
        with open(other, "r+b") as fh:                             # same size and time, other audio
            fh.truncate(os.path.getsize(moved))
        os.utime(other, ns=(os.stat(moved).st_atime_ns, os.stat(moved).st_mtime_ns))
        assert st.lookup(other, key) is None

        with open(moved, "ab") as fh:                              # a changed file is analysed again
            fh.write(b"\0")
        assert st.lookup(moved, key) is None

        assert not st.put(FileResult(path=other, verdict=ERROR, reasons=["boom"]), key)  # errors are not kept
        assert st.lookup(other, key) is None

        st.put(analyze_file(other), key)
        monkeypatch.setattr(store, "DETECTOR_VERSION", store.DETECTOR_VERSION + 1)
        assert st.lookup(other, key).saved["outdated"]             # made by older detection rules


@needs_ffmpeg
def test_graphs_are_trimmed_least_recently_viewed_first(tmp_path):
    key = settings_key()
    paths = []
    with ResultStore(graphs_mb=0.000001) as st:                    # 1 byte: nothing fits
        for i in range(3):
            p = ffmpeg_write(tmp_path / f"t{i}.wav", music_like(48000, 3, seed=i), 48000, ["-c:a", "pcm_s24le"])
            r = analyze_file(p, keep_plot=True)
            st.graphs_limit = 1e9
            st.put(r, key, plot=r.plot)
            paths.append(p)
        st.get_plot(paths[0], key)                                 # viewed last: kept longest
        total = st.stats()["graph_bytes"]
        st.graphs_limit = total * 0.75
        st.trim_graphs()
        assert st.get_plot(paths[0], key) is not None
        assert st.get_plot(paths[1], key) is None                  # oldest view dropped first
        assert all(st.lookup(p, key) is not None for p in paths)   # results always stay


def test_damaged_database_is_set_aside(tmp_path):
    db = tmp_path / "results.sqlite"
    db.write_bytes(b"this is not a database" * 100)
    with ResultStore(str(db)) as st:
        assert st.stats()["results"] == 0
    assert any(n.startswith("results.sqlite.damaged-") for n in os.listdir(tmp_path))


@needs_ffmpeg
def test_cli_reuses_saved_results(tmp_path, capsys, monkeypatch):
    monkeypatch.setenv("REALHIRES_SETTINGS", str(tmp_path / "s.json"))
    folder = tmp_path / "music"
    folder.mkdir()
    for i in range(3):
        ffmpeg_write(folder / f"t{i}.flac", music_like(96000, 4, top_hz=30000, seed=i), 96000,
                     ["-c:a", "flac", "-sample_fmt", "s32"])
    run = lambda *a: (cli.main([str(folder), "--no-color", "-j", "2", *a]), capsys.readouterr().out)[1]  # noqa: E731
    assert "0 file(s) unchanged since an earlier scan were not analysed again, 3 analysed" in run()
    out = run("-v")
    assert "3 file(s) unchanged since an earlier scan were not analysed again, 0 analysed" in out
    assert "Saved result from" in out
    assert "0 file(s) unchanged" in run("--rescan")
    assert "Saved results:" not in run("--no-cache")
    ffmpeg_write(folder / "new.flac", music_like(96000, 4, top_hz=30000, seed=9), 96000,
                 ["-c:a", "flac", "-sample_fmt", "s32"])
    assert "3 file(s) unchanged since an earlier scan were not analysed again, 1 analysed" in run()
    monkeypatch.setattr(store, "DETECTOR_VERSION", store.DETECTOR_VERSION + 1)
    out = run()
    assert "4 of them were checked by older detection rules" in out
    assert "0 file(s) unchanged" in run("--rescan-outdated")
    assert "Deleted 4 saved result(s)" in (cli.main(["--forget-results"]), capsys.readouterr().out)[1]
