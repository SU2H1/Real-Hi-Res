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
    lenient.ultrasonic_khz = 15.0     # ... and accept little ultrasonic content
    lenient.steep_drop_db = 149.0
    assert analyze_file(path, settings=lenient).verdict == REAL


@needs_ffmpeg
def test_cli_set_and_list_settings(library, capsys, tmp_path, monkeypatch):
    monkeypatch.setenv("REALHIRES_SETTINGS", str(tmp_path / "none.json"))
    assert main(["--list-settings"]) == 0
    assert "cliff_db" in capsys.readouterr().out
    rc = main([library["fake_up_44_to_96"], "--no-color", "--set", "cliff_db=149",
               "--set", "steep_drop_db=149", "--set", "ultrasonic_khz=15"])
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
import sys
sys.path.insert(0, {str(Path(__file__).resolve().parents[1])!r})
def run():
    import tkinter as tk
    from realhires import gui, settings
    root = tk.Tk()
    folder = {str(Path(library['real_24_96']).parent)!r}
    app = gui.App(root)
    app._add_source(folder)
    app._add_source(folder)  # adding twice must not duplicate
    def listed():
        if app.listing:
            return root.after(100, listed)
        rows = app.tree.get_children()
        n = len(app.all_files())
        assert len(app.sources) == 1 and n >= 10, (app.sources, n)
        assert len(rows) == n, "listed files are not all shown before scanning"
        assert all(app.tree.set(i, "verdict") == "Not scanned" for i in rows)
        assert app.src_tree.set(folder, "files") == f"{{n}} files"
        assert "ready" in app.status_var.get()
        app.start_scan()
        root.after(200, check)
    def check():
        if app.scanning or not app.results:
            return root.after(200, check)
        root.after(600, verify)
    def verify():
        root.update()
        dialogs = [w for w in root.winfo_children() if isinstance(w, gui.SummaryDialog)]
        listed = app.tree.get_children()
        assert listed, "nothing listed"
        assert all(app.results[i].is_problem for i in listed), "default filter shows non-problems"
        app.tree.selection_set(listed[0]); root.update()
        assert app.badge.cget("text"), "dashboard not filled"
        assert app.cache.get(listed[0])["spectrogram"], "no spectrogram"
        assert " of " in app.src_tree.set(folder, "problems"), "source problem count missing"
        new = settings.Settings(); new.colormap = "gray"
        app.apply_settings(new)
        assert settings.load().colormap == "gray"
        print("OK", len(dialogs), len(listed))
        app.quit_app()
    root.after(300, listed)
    root.mainloop()
if __name__ == "__main__":
    run()
""")
    env = dict(os.environ, REALHIRES_SETTINGS=str(tmp_path / "gui-settings.json"))
    proc = subprocess.run([sys.executable, str(script)], capture_output=True, text=True,
                          timeout=180, env=env)
    assert proc.returncode == 0, proc.stderr
    assert proc.stdout.startswith("OK 1 ")  # one summary pop-up was shown


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
