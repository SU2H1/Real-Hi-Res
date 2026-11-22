"""Settings, summaries, spectrograms and the GUI."""

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

import numpy as np
import pytest

from realhires import analysis, settings
from realhires.analysis import decode_spectrogram
from realhires.cli import main
from realhires.imaging import spectrogram_png, spectrogram_rgb
from realhires.verdict import FAKE, REAL, FileResult, analyze_file, verdict_breakdown

needs_ffmpeg = pytest.mark.skipif(shutil.which("ffmpeg") is None, reason="ffmpeg not installed")


# ---------------------------------------------------------------- settings ---
def test_settings_defaults_match_detection_constants():
    s = settings.Settings()
    c = analysis.detection_params(s)
    d = analysis.detection_params(None)
    assert vars(c) == pytest.approx(vars(d))


def test_settings_roundtrip_and_validation(tmp_path):
    path = tmp_path / "s.json"
    s = settings.Settings()
    s.cliff_db = 33.0
    s.colormap = "viridis"
    s.shown_verdicts = ["FAKE LOSSLESS"]
    settings.save(s, str(path))
    t = settings.load(str(path))
    assert (t.cliff_db, t.colormap, t.shown_verdicts) == (33.0, "viridis", ["FAKE LOSSLESS"])
    # broken / out-of-range / unknown values fall back to defaults instead of crashing
    path.write_text(json.dumps({"cliff_db": "x", "jobs": -5, "colormap": "nope", "zzz": 1}))
    u = settings.load(str(path))
    assert (u.cliff_db, u.colormap, u.jobs) == (25.0, "inferno", 0)  # 0 = automatic
    path.write_text("{not json")
    assert settings.load(str(path)).cliff_db == 25.0
    with pytest.raises(ValueError):
        s.set_value("cliff_db", "1000")
    with pytest.raises(ValueError):
        s.set_value("no_such_setting", "1")
    s.set_value("recursive", "no")
    assert s.recursive is False
    assert ".flac" in s.extension_set()


@needs_ffmpeg
def test_settings_change_detection(library):
    path = library["fake_up_44_to_96"]
    assert analyze_file(path).verdict == FAKE
    lenient = settings.Settings()
    lenient.cliff_db = 149.0          # demand an impossible cliff ...
    lenient.deep_cliff_db = 199.0     # ... on both cliff tests ...
    lenient.ultrasonic_khz = 15.0     # ... and accept little ultrasonic content
    lenient.steep_drop_db = 149.0
    assert analyze_file(path, settings=lenient).verdict == REAL


@needs_ffmpeg
def test_cli_set_and_list_settings(library, capsys, tmp_path, monkeypatch):
    monkeypatch.setenv("REALHIRES_SETTINGS", str(tmp_path / "none.json"))
    assert main(["--list-settings"]) == 0
    assert "cliff_db" in capsys.readouterr().out
    rc = main([library["fake_up_44_to_96"], "--no-color", "--set", "cliff_db=149",
               "--set", "deep_cliff_db=199", "--set", "steep_drop_db=149", "--set", "ultrasonic_khz=15"])
    assert rc == 0 and "REAL HI-RES" in capsys.readouterr().out
    assert main(["x", "--set", "bogus=1"]) == 2


# --------------------------------------------------------------- summaries ---
def test_verdict_breakdown_percentages():
    rs = [FileResult(path=str(i), verdict=v) for i, v in
          enumerate([FAKE, FAKE, REAL, "FAKE LOSSLESS"])]
    got = verdict_breakdown(rs)
    assert got[0] == ("FAKE LOSSLESS", 1, 25.0)
    assert ("FAKE HI-RES", 2, 50.0) in got and ("REAL HI-RES", 1, 25.0) in got


# ------------------------------------------------------------ spectrogram ---
@needs_ffmpeg
def test_spectrogram_shows_cliff(library, tmp_path):
    r = analyze_file(library["fake_up_44_to_96"], keep_plot=True)
    sg = r.plot["spectrogram"]
    db = decode_spectrogram(sg)
    assert db.shape[1] == analysis.SPECTRO_BANDS and db.shape[0] >= 10
    f = np.linspace(0, sg["nyquist"], db.shape[1], endpoint=False)
    below = db[:, (f > 15000) & (f < 20000)].mean()
    above = db[:, (f > 25000) & (f < 45000)].mean()
    assert below - above > 60  # nothing above the 22.05 kHz cliff
    assert abs(db.shape[0] * sg["seconds_per_column"] - r.analysed_seconds) < 1.0
    rgb = spectrogram_rgb(sg, 300, 120)
    assert rgb.shape == (120, 300, 3) and rgb.dtype == np.uint8
    png = spectrogram_png(sg, 400, 200, markers=[(22050, (0, 200, 255))])
    assert png.startswith(b"\x89PNG")
    out = tmp_path / "sg"
    assert main([library["fake_up_44_to_96"], "--no-color", "--no-settings",
                 "--spectrogram", str(out)]) == 0
    assert len(list(out.glob("*.png"))) == 1


# --------------------------------------------------------------------- GUI ---
def _gui_available():
    if not os.environ.get("DISPLAY") and sys.platform.startswith("linux"):
        return False
    try:
        import tkinter

        tkinter.Tk().destroy()
        return True
    except Exception:  # noqa: BLE001
        return False


@needs_ffmpeg
@pytest.mark.skipif(not _gui_available(), reason="needs Tkinter and a display")
def test_gui_scan_summary_and_settings(library, tmp_path):
    script = tmp_path / "drive.py"
    script.write_text(f"""
import faulthandler, os, signal, sys, threading, time
sys.path.insert(0, {str(Path(__file__).resolve().parents[1])!r})
# Module level, so the spawned worker processes (which import this file) can dump their stacks too.
if hasattr(signal, "SIGUSR1"):
    faulthandler.register(signal.SIGUSR1, all_threads=True)
def watchdog():
    import multiprocessing
    print("WATCHDOG: the GUI test hangs; stacks of every process follow", file=sys.stderr, flush=True)
    for child in multiprocessing.active_children():
        print(f"--- worker process {{child.pid}} ---", file=sys.stderr, flush=True)
        if hasattr(signal, "SIGUSR1"):
            os.kill(child.pid, signal.SIGUSR1)
    time.sleep(3)
    print("--- GUI process ---", file=sys.stderr, flush=True)
    faulthandler.dump_traceback(all_threads=True)
    os._exit(3)
def run():
    dog = threading.Timer(180, watchdog)
    dog.daemon = True  # must not keep a finished test alive
    dog.start()
    import tkinter as tk
    from realhires import gui, settings
    root = tk.Tk()
    def fail(exc, val, tb):  # an assertion in a Tk callback must end the test, not hang it
        import traceback, os
        traceback.print_exception(exc, val, tb)
        os._exit(1)
    root.report_callback_exception = fail
    folder = {str(Path(library['real_24_96']).parent)!r}
    app = gui.App(root)
    app._add_source(folder)
    app._add_source(folder)  # adding twice must not duplicate
    def listed():
        if app.listing or app.lookup_busy:  # the status says "ready" once saved results were looked up
            return root.after(100, listed)
        rows = app.tree.get_children()
        n = len(app.all_files())
        assert len(app.sources) == 1 and n >= 10, (app.sources, n)
        assert len(rows) == n, "listed files are not all shown before scanning"
        assert all(app.tree.set(i, "verdict") == "Not scanned" for i in rows)
        assert app.src_tree.set(folder, "files") == f"{{n}} files"
        assert "ready" in app.status_var.get()
        # Settings -> Fixing -> After a scan; copies into a temp folder keep the test files intact
        app.cfg.fix_after_scan = "Fix them automatically"
        app.cfg.fix_output = "Saving a copy into a folder"
        app.cfg.fix_folder = {str(tmp_path / "fixed")!r}
        app.start_scan()
        root.after(200, check)
    def check():
        if app.scanning or not app.results:
            return root.after(200, check)
        fixers = [w for w in root.winfo_children() if isinstance(w, gui.FixDialog)]
        if not (fixers and (fixers[0].running or fixers[0].done_paths)):  # starts shortly after the scan
            state["waits"] = state.get("waits", 0) + 1
            assert state["waits"] < 100, "no automatic fix after the scan"
            return root.after(100, check)
        print("auto-fix started", flush=True)
        root.after(200, fix_done)
    def fix_done():
        # let the automatic fix finish before the next scans, so they do not compete for the CPU
        fixer = [w for w in root.winfo_children() if isinstance(w, gui.FixDialog)][0]
        if fixer.running:
            return root.after(200, fix_done)
        print("auto-fix finished", flush=True)
        root.after(300, verify)
    def verify():
        root.update()
        dialogs = [w for w in root.winfo_children() if isinstance(w, gui.SummaryDialog)]
        fixers = [w for w in root.winfo_children() if isinstance(w, gui.FixDialog)]
        assert len(fixers) == 1 and fixers[0].plans and fixers[0].done_paths, "no automatic fix after the scan"
        assert all(p.output.startswith({str(tmp_path / "fixed")!r}) for p in fixers[0].plans)
        if any(r.verdict == "FAKE LOSSLESS" and r.sample_rate == 44100 for r in fixers[0].results):
            assert "made from MP3/AAC but already at CD resolution" in fixers[0].note.cget("text")
        app.cfg.fix_after_scan = "Do nothing"
        fixers[0].destroy()
        listed = app.tree.get_children()
        assert listed, "nothing listed"
        assert all(app.results[i].is_problem for i in listed), "default filter shows non-problems"
        app.tree.selection_set(listed[0]); root.update()
        assert app.badge.cget("text"), "dashboard not filled"
        assert app.cache.get(listed[0])["spectrogram"], "no spectrogram"
        assert " of " in app.src_tree.set(folder, "problems"), "source problem count missing"
        # album view, drill-down and back
        app.set_view("By album"); root.update()
        assert app.cards.groups, "no album cards"
        g = app.cards.groups[0]
        app.open_group(g, "album"); root.update()
        assert app.view_var.get() == "Files" and len(app.tree.get_children()) == g.total
        app.close_group(); root.update()
        assert app.view_var.get() == "By album"
        app.set_view("Files"); root.update()
        # browser-style find
        app.open_find(); app.find_var.set("upsampled"); root.update()
        n = len(app.find_matches)
        assert n >= 1 and app.find_count.cget("text") == f"1 of {{n}}"
        app.find_step(1); root.update()
        assert app.find_count.cget("text") == f"{{min(2, n) if n > 1 else 1}} of {{n}}"
        app.close_find()
        # the Search box waits for typing to pause instead of refreshing on every key
        shown = len(app.tree.get_children())
        app.search_var.set("zzzz-no-such-file")
        assert app._search_job is not None and len(app.tree.get_children()) == shown, "search ran on a key"
        app._run_search(); root.update()
        assert not app.tree.get_children(), "search did not filter"
        app.search_var.set(""); app._run_search(); root.update()
        assert len(app.tree.get_children()) == shown
        # "Fake lossless + Fake hi-res, Ctrl/Cmd+A, Fix": select-all takes every listed file
        app.set_filters(["FAKE LOSSLESS", "FAKE HI-RES"]); root.update()
        fakes = app.tree.get_children()
        assert fakes and app.select_all() == "break"; root.update()
        assert set(app.tree.selection()) == set(fakes), "select-all missed files"
        assert str(app.fix_btn.cget("state")) == "normal", "Fix not enabled for a fake selection"
        new = settings.Settings(); new.colormap = "gray"
        app.apply_settings(new)
        assert settings.load().colormap == "gray"
        state["dialogs"], state["listed"] = len(dialogs), len(listed)
        state["errors"] = {{p for p, r in app.results.items() if r.verdict == "ERROR"}}
        # saved results: listing the folder again shows every result without a scan
        print("relisting", flush=True)
        app.clear(); app._add_source(folder)
        root.after(100, saved)
    def saved():
        if app.listing or app.lookup_busy:
            return root.after(100, saved)
        missing = {{f for f in app.all_files() if f not in app.results}}
        assert missing == state["errors"], ("not reused", sorted(missing)[:3])
        assert all(r.saved and not r.saved["outdated"] for r in app.results.values())
        assert "checked before" in app.status_var.get(), app.status_var.get()
        app.set_filters(["FAKE HI-RES"]); root.update()
        p = app.tree.get_children()[0]
        app.tree.selection_set(p); root.update()
        assert app.cache.get(p)["spectrogram"], "saved graphs not used"
        assert "Saved result from" in app.text.get("1.0", "end")
        # graphs that were not saved are drawn on demand
        from realhires.store import ResultStore
        with ResultStore() as st:
            st.db.execute("UPDATE results SET graphs=NULL, spectro=NULL, gbytes=0")
        app.cache.clear(); app.show_details()
        assert app.spectrogram.waiting, "no on-demand graphs"
        state["path"] = p
        root.after(200, drawn)
    def drawn():
        p = state["path"]
        if app.cache.get(p) is None:
            return root.after(200, drawn)
        assert app.cache.get(p)["spectrogram"] and not app.spectrogram.waiting
        print("re-scan", flush=True)
        app.rescan_selected()
        assert app.scanning
        root.after(200, rescanned)
    def rescanned():
        if app.scanning:
            return root.after(200, rescanned)
        p = state["path"]
        assert app.results[p].saved is None, "re-scan reused the saved result"
        assert sum(1 for r in app.results.values() if r.saved) == len(app.results) - 1
        # a full scan now analyses nothing: every file has a saved result
        state["t"] = time.time()
        app.start_scan()
        root.after(100, full)
    def full():
        if app.scanning:
            return root.after(100, full)
        n = len(app.all_files()) - len(state["errors"])
        assert sum(1 for r in app.results.values() if r.saved) == n, "scan did not reuse saved results"
        assert app.done == len(app.all_files()) and not app.scan_order
        assert "from saved results" in app.status_var.get(), app.status_var.get()
        print("OK", state["dialogs"], state["listed"])
        app.quit_app()
    state = {{}}
    root.after(300, listed)
    root.mainloop()
if __name__ == "__main__":
    run()
""")
    env = dict(os.environ, REALHIRES_SETTINGS=str(tmp_path / "gui-settings.json"))
    # Output goes to files, not pipes: worker processes that outlive a crash would keep pipes
    # open and hide the watchdog's stack dumps until the timeout.
    out, err = tmp_path / "gui.out", tmp_path / "gui.err"
    with open(out, "w") as fo, open(err, "w") as fe:
        proc = subprocess.Popen([sys.executable, str(script)], stdout=fo, stderr=fe, env=env)
        try:
            rc = proc.wait(timeout=240)
        except subprocess.TimeoutExpired:
            proc.kill()
            rc = "killed after 240 s"
    log = out.read_text(errors="replace") + err.read_text(errors="replace")
    assert rc == 0, log
    assert out.read_text().splitlines()[-1].startswith("OK 1 "), log  # one summary pop-up was shown


# ---------------------------------------------------------- console output ---
@needs_ffmpeg
def test_cli_survives_legacy_console_encoding(library, tmp_path):
    """A Windows cp1252 console cannot encode '→', '█' or Japanese names."""
    src = Path(library["fake_up_44_to_96"])
    jp = tmp_path / "ハイレゾ 偽物 テスト.flac"
    shutil.copy(src, jp)
    env = dict(os.environ, PYTHONIOENCODING="cp1252", REALHIRES_SETTINGS=str(tmp_path / "s.json"))
    proc = subprocess.run([sys.executable, "-m", "realhires", str(jp), "--no-color"],
                          capture_output=True, env=env, timeout=120,
                          cwd=str(Path(__file__).resolve().parents[1]))
    assert proc.returncode == 0, proc.stderr.decode("utf-8", "replace")
    out = proc.stdout.decode("utf-8")  # redirected output is written as UTF-8
    assert "ハイレゾ 偽物 テスト.flac" in out and "FAKE HI-RES" in out


def test_ascii_console_fallbacks(monkeypatch):
    import io

    from realhires import report

    monkeypatch.setattr(sys, "stdout", io.TextIOWrapper(io.BytesIO(), encoding="ascii"))
    assert report.arrow() == "->" and report.bar_char() == "#"
    r = FileResult(path="/x/a.flac", verdict=FAKE, summary="Upsampled from 44.1 kHz")
    assert "-> Upsampled" in report.row(r, color=False)


# ------------------------------------------------------------ parallelism ---
def test_pool_split_respects_windows_limit():
    from realhires.pool import MultiProcessPool, effective_jobs, split_workers

    assert split_workers(64, 61) == [32, 32]
    assert split_workers(130, 61) == [44, 43, 43] and sum(split_workers(500, 61)) == 500
    assert all(n <= 61 for n in split_workers(512, 61))
    assert split_workers(8, 61) == [8]
    assert effective_jobs(0) >= 1 and effective_jobs(10_000) == 512
    import multiprocessing

    with MultiProcessPool(3, mp_context=multiprocessing.get_context("spawn"), limit=2) as pool:  # 2 + 1
        assert pool.sizes == [2, 1]
        assert sorted(f.result() for f in [pool.submit(abs, -i) for i in range(6)]) == list(range(6))


def test_settings_jobs_zero_is_automatic():
    s = settings.Settings()
    assert s.jobs == 0
    s.set_value("jobs", "200")
    assert s.jobs == 200
    with pytest.raises(ValueError):
        s.set_value("jobs", "513")


def test_listing_streams_batches(tmp_path):
    import queue
    import threading

    from realhires.listing import list_source

    for i in range(1200):
        (tmp_path / f"t{i:04d}.flac").write_bytes(b"")
    (tmp_path / "notes.txt").write_text("x")
    q = queue.Queue()
    list_source(str(tmp_path), settings.Settings(), q, threading.Event())
    msgs = list(q.queue)
    assert msgs[-1] == ("listed", str(tmp_path), None)
    batches = [m[2] for m in msgs[:-1]]
    assert len(batches) >= 3 and sum(len(b) for b in batches) == 1200  # streamed, not all at once


# ------------------------------------------------------------------- fixing ---
@needs_ffmpeg
def test_fix_upsampled_and_padded(library, tmp_path):
    from realhires.fix import plan_fix, run_fix

    cfg = settings.Settings()
    cfg.fix_output, cfg.fix_folder = "Saving a copy into a folder", str(tmp_path / "out")
    r = analyze_file(library["fake_up_44_to_96"])
    plan, why = plan_fix(r, cfg)
    assert plan and plan.description == "24/96 → 16/44.1", why
    out = run_fix(plan, cfg)
    assert out.ok, out.message
    assert out.new_result.verdict == "NOT HI-RES" and out.new_result.claimed == "16/44.1"
    assert out.bytes_after < out.bytes_before
    assert os.path.exists(library["fake_up_44_to_96"])  # original untouched
    cfg.fix_bits = "As detected"
    plan2, _ = plan_fix(r, cfg)
    assert plan2.description == "24/96 → 24/44.1"
    p = analyze_file(library["fake_padded_16_in_24"])
    plan3, _ = plan_fix(p, cfg)
    assert plan3.description == "24/96 → 16/96" and not plan3.resample
    assert run_fix(plan3, cfg).new_result.effective_bits == 16
    plan4, why4 = plan_fix(analyze_file(library["real_24_96"]), cfg)
    assert plan4 is None and "need fixing" in why4


def test_fix_never_cuts_below_a_safe_rate():
    from realhires.fix import safe_source_rate

    r = FileResult(path="x.flac", verdict=FAKE, sample_rate=96000, channels=2, claimed_bits=24,
                   effective_bits=24, upsampled_from=44100)
    r.cliffs_db, r.deep_cliffs_db, r.upsample_mid_hz = {44100: 60.0, 48000: 70.0}, {}, 21100.0
    assert safe_source_rate(r) == 44100                 # clear cliff at 22.05 kHz: cut there
    r.upsample_mid_hz = 23000.0                         # music still there at 23 kHz
    assert safe_source_rate(r) == 48000                 # -> keep 48 kHz, nothing is lost
    r.cliffs_db = {44100: 15.0, 48000: 12.0}            # weak evidence everywhere
    assert safe_source_rate(r) == 96000                 # -> do not resample at all


def _pcm(path, channels=2):
    from realhires.probe import iter_pcm

    return np.concatenate(list(iter_pcm(str(path), channels)))


@needs_ffmpeg
def test_fix_restores_16_bit_master_bit_exactly(library, tmp_path):
    from realhires.fix import plan_fix, run_fix

    master = Path(library["fake_up_48_to_96_swr"]).parent / "m48.wav"
    fakes = {}
    for rate in (96000, 192000):  # FFmpeg's default resampler, 2x and 4x
        f = tmp_path / f"fake_{rate}.flac"
        subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", str(master), "-ar", str(rate), "-c:a", "flac",
                        "-sample_fmt", "s32", "-metadata", "title=曲名", str(f)], check=True)
        fakes[rate] = f
    cfg = settings.Settings()
    cfg.fix_output, cfg.fix_folder = "Saving a copy into a folder", str(tmp_path / "out")
    want = _pcm(master)
    for rate, f in fakes.items():
        plan, why = plan_fix(analyze_file(str(f)), cfg)
        assert plan and plan.exact_ratio == rate // 48000, why
        out = run_fix(plan, cfg)
        assert out.ok and "bit-exactly" in out.message, out.message
        got = _pcm(out.output)
        assert got.shape == want.shape and np.array_equal(got, want)  # the master itself
        tags = subprocess.run(["ffprobe", "-v", "error", "-show_entries", "format_tags=title", "-of", "json",
                               out.output], capture_output=True, text=True, encoding="utf-8").stdout
        assert "曲名" in tags
    # A volume change after the upsample leaves no untouched sample: the phase check
    # must refuse it and the file is resampled as before (on every FFmpeg build).
    scaled = tmp_path / "scaled_96.flac"
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", str(master), "-af", "aresample=96000,volume=0.9",
                    "-c:a", "flac", "-sample_fmt", "s32", str(scaled)], check=True)
    plan, _ = plan_fix(analyze_file(str(scaled)), cfg)
    assert plan.exact_ratio == 2
    out = run_fix(plan, cfg)
    assert out.ok and "bit-exactly" not in out.message, out.message


@needs_ffmpeg
def test_fix_keeps_gapless_joins(tmp_path):
    from conftest import ffmpeg_write, hq_resample, music_like
    from realhires.fix import _resample_filter, plan_fix, prepare_gapless, run_fix

    # One continuous 44.1 kHz recording, upsampled to 24/96 as a whole and cut into
    # three tracks at points that are not on the 44.1 kHz grid.
    album = ffmpeg_write(tmp_path / "album44.wav", music_like(44100, 9, seed=11), 44100, ["-c:a", "pcm_s16le"])
    up = tmp_path / "album96.wav"
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", album, *hq_resample(96000), "-c:a", "pcm_s24le",
                    str(up)], check=True)
    x = _pcm(up)
    cuts = [0, 300007, 600013, x.shape[0]]
    d = tmp_path / "Album"
    d.mkdir()
    for i in range(3):
        ffmpeg_write(d / f"0{i + 1} track.wav", x[cuts[i]:cuts[i + 1]] / 2.0 ** 31, 96000, ["-c:a", "pcm_s24le"])
    cfg = settings.Settings()
    cfg.fix_bits = "As detected"   # 24-bit output: no dither, so the joins can be compared exactly
    cfg.fix_output, cfg.fix_folder = "Saving a copy into a folder", str(tmp_path / "out")
    results = [analyze_file(str(f)) for f in sorted(d.iterdir())]
    plans = [plan_fix(r, cfg)[0] for r in results]
    assert all(p and p.target_rate == 44100 for p in plans)
    assert prepare_gapless(plans, results) == 2
    outs = [run_fix(p, cfg) for p in plans]
    assert all(o.ok for o in outs) and all("gapless" in o.message for o in outs), [o.message for o in outs]
    joined = np.concatenate([_pcm(o.output) for o in outs]).astype(np.float64)
    whole = tmp_path / "whole.wav"   # the reference: the album converted in one piece, same filter
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", str(up), "-af", _resample_filter(plans[0]),
                    "-c:a", "pcm_s24le", str(whole)], check=True)
    ref = _pcm(whole).astype(np.float64)
    assert joined.shape == ref.shape
    err = np.abs(joined - ref).max(axis=1) / 2.0 ** 31
    assert err.max() < 10 ** (-100 / 20), (20 * np.log10(err.max()), int(err.argmax()))  # 24-bit rounding
    # A silent join needs nothing.
    assert prepare_gapless([plan_fix(results[0], cfg)[0]], results[:1]) == 0


@needs_ffmpeg
def test_fix_replace_keeps_backup_and_tags(library, tmp_path):
    from realhires.fix import plan_fix, run_fix
    from realhires.probe import probe

    src = tmp_path / "Track テスト.flac"
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", library["fake_up_96_to_192"], "-c:a", "flac",
                    "-sample_fmt", "s32", "-metadata", "title=曲名", "-metadata", "artist=A", str(src)],
                   check=True)
    cfg = settings.Settings()
    cfg.fix_output = "Replace the file"
    cfg.fix_backup = "Keep it as <name>.bak next to the file"
    plan, _ = plan_fix(analyze_file(str(src)), cfg)
    assert plan.description == "24/192 → 24/96"
    out = run_fix(plan, cfg)
    assert out.ok and out.output == str(src)
    assert (tmp_path / "Track テスト.flac.bak").exists()
    info = probe(str(src))
    assert info.sample_rate == 96000 and info.claimed_bits == 24
    tags = subprocess.run(["ffprobe", "-v", "error", "-show_entries", "format_tags=title,artist",
                           "-of", "json", str(src)], capture_output=True, text=True, encoding="utf-8").stdout
    assert "曲名" in tags and '"artist": "A"' in tags


@needs_ffmpeg
def test_cli_fix_dry_run_and_convert(library, tmp_path, capsys):
    folder = tmp_path / "in"
    folder.mkdir()
    shutil.copy(library["fake_up_44_to_96"], folder / "a.flac")
    shutil.copy(library["real_24_96"], folder / "b.flac")
    assert main([str(folder), "--no-color", "--no-settings", "--fix", "--dry-run"]) == 0
    out = capsys.readouterr().out
    assert "would fix in place 1 file" in out and "24/96 → 16/44.1" in out and "(same file)" in out
    assert main([str(folder), "--no-color", "--no-settings", "--fix", "--fix-dir", str(tmp_path / "fixed"),
                 "-j", "0"]) == 0
    assert "1 fixed, 0 failed" in capsys.readouterr().out
    assert [p.name for p in (tmp_path / "fixed").iterdir()] == ["a [16-44.1].flac"]


@needs_ffmpeg
def test_fix_in_place_default_moves_original_to_backup_folder(library, tmp_path):
    """Default 'fix': the file itself now has its real properties; the
    original goes to 'Real-Hi-Res backups', which later scans ignore."""
    from realhires.cli import collect
    from realhires.fix import BACKUP_DIR_NAME, plan_fix, run_fix
    from realhires.probe import probe

    f = tmp_path / "Album" / "01 Song.flac"
    f.parent.mkdir()
    shutil.copy(library["fake_up_44_to_96"], f)
    cfg = settings.Settings()  # defaults: replace the file, backup folder
    plan, _ = plan_fix(analyze_file(str(f)), cfg)
    assert plan.replace_original and plan.output == str(f) and plan.backup == "folder"
    out = run_fix(plan, cfg)
    assert out.ok, out.message
    info = probe(str(f))
    assert (info.sample_rate, info.claimed_bits) == (44100, 16)  # the file's properties are fixed
    backup = f.parent / BACKUP_DIR_NAME / "01 Song.flac"
    assert backup.exists() and probe(str(backup)).sample_rate == 96000
    assert collect([str(tmp_path)], recursive=True) == [str(f)]  # backups are never rescanned
    assert analyze_file(str(f)).verdict == "NOT HI-RES"


def test_old_fix_settings_are_migrated():
    s = settings.Settings.from_dict({"fix_output": "Replace the original (keep .bak)"})
    assert s.fix_output == "Replace the file" and s.fix_backup.startswith("Keep")
    assert settings.Settings.from_dict({"fix_output": "Into a folder"}).fix_output == "Saving a copy into a folder"


# ------------------------------------------------------- albums / artists ---
def _fr(path, verdict, **tags):
    return FileResult(path=path, verdict=verdict, tags=tags)


def test_group_by_album_and_artist():
    from realhires.library import group_results, sort_groups

    rs = [
        _fr("/m/A/X/1.flac", FAKE, artist="Aiko", album_artist="Aiko", album="Blue", track="2"),
        _fr("/m/A/X/2.flac", REAL, artist="Aiko", album_artist="Aiko", album="Blue", track="1"),
        _fr("/m/A/Y/1.flac", "SUSPICIOUS", artist="Aiko", album="Red"),
        _fr("/m/B/Z/1.flac", REAL, artist="Bob feat. C", album_artist="Bob", album="Green"),
        _fr("/music/Various/Live/1.wav", "FAKE LOSSLESS"),  # untagged: folder names are used
    ]
    albums = {g.title: g for g in group_results(rs, "album")}
    assert set(albums) == {"Blue", "Red", "Green", "Live"}
    blue = albums["Blue"]
    assert (blue.subtitle, blue.total, blue.fake, blue.problems) == ("Aiko", 2, 1, 1)
    assert blue.paths == ["/m/A/X/2.flac", "/m/A/X/1.flac"]  # sorted by track number
    assert albums["Live"].subtitle == "Various" and albums["Live"].problem_pct == 100.0
    artists = {g.title: g for g in group_results(rs, "artist")}
    assert artists["Aiko"].total == 3 and artists["Aiko"].subtitle == "2 albums"
    assert artists["Aiko"].fake == 1 and artists["Aiko"].suspicious == 1
    assert "Bob" in artists  # album artist preferred over track artist
    order = [g.title for g in sort_groups(list(albums.values()))]
    assert order[0] == "Live" and order[-1] == "Green"  # most fake first
    assert [g.title for g in sort_groups(list(albums.values()), "Name")] == ["Blue", "Green", "Live", "Red"]


@needs_ffmpeg
def test_tags_and_covers(library, tmp_path):
    from realhires.covers import cover_png
    from realhires.probe import probe

    icon = str(Path(__file__).resolve().parents[1] / "realhires" / "assets" / "icon.png")
    tagged = tmp_path / "t.flac"
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", library["real_24_96"], "-i", icon,
                    "-map", "0:a", "-map", "1:v", "-c:a", "copy", "-c:v", "copy",
                    "-disposition:v", "attached_pic", "-metadata", "ARTIST=宇多田", "-metadata",
                    "ALBUM=First", "-metadata", "TRACKNUMBER=3", str(tagged)], check=True)
    tags = probe(str(tagged)).tags
    assert tags["artist"] == "宇多田" and tags["album"] == "First" and tags["track"] == "3"
    png = cover_png([str(tagged)], 64)
    assert png and png.startswith(b"\x89PNG")
    folder = tmp_path / "album"
    folder.mkdir()
    plain = folder / "a.flac"
    shutil.copy(library["real_24_96"], plain)
    assert cover_png([str(plain)], 64) is None
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", icon, str(folder / "folder.jpg")], check=True)
    assert cover_png([str(plain)], 64).startswith(b"\x89PNG")  # folder.jpg is used


def test_fix_default_is_replace_even_for_old_saved_settings():
    assert settings.Settings().fix_output == "Replace the file"
    old = {"fix_output": "Saving a copy next to it"}  # saved by 1.5.0 (migrated 1.4.0 default)
    assert settings.Settings.from_dict(old).fix_output == "Replace the file"
    new = dict(old, _version=settings.SETTINGS_VERSION)  # a deliberate choice after 1.6.0 stays
    assert settings.Settings.from_dict(new).fix_output == "Saving a copy next to it"


def test_fix_explains_files_it_leaves_alone():
    from realhires.fix import plan_fix, plan_fixes, skip_group, skip_summary

    cfg = settings.Settings()
    lossy = FileResult(path="/m/a.flac", verdict="FAKE LOSSLESS", codec="flac", sample_rate=44100,
                       claimed_bits=16, effective_bits=16, lossy_cutoff_hz=16000.0)
    plan, why = plan_fix(lossy, cfg)
    assert plan is None and "nothing to convert" in why and skip_group(lossy, why) == "lossy"
    padded = FileResult(path="/m/b.flac", verdict="FAKE HI-RES", codec="flac", sample_rate=96000,
                        claimed_bits=24, effective_bits=16)
    plans, skipped = plan_fixes([lossy, padded, FileResult(path="/m/c.flac", verdict="REAL HI-RES")], cfg)
    assert [p.source for p in plans] == ["/m/b.flac"] and [r.path for r, _ in skipped] == ["/m/a.flac"]
    text = skip_summary(skipped + [(lossy, why)])
    assert text.startswith("2 file(s) are not changed: 2 made from MP3/AAC but already at CD resolution")
    assert skip_group(padded, "Kept 96 kHz instead of the estimated 48 kHz: ...") == "uncertain"
    assert skip_summary([]) == ""
