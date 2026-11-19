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
    path.write_text(json.dumps({"cliff_db": "x", "jobs": 0, "colormap": "nope", "zzz": 1}))
    u = settings.load(str(path))
    assert (u.cliff_db, u.colormap) == (25.0, "inferno") and u.jobs >= 1
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
    app = gui.App(root, initial=[{str(Path(library['real_24_96']).parent)!r}])
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
        new = settings.Settings(); new.colormap = "gray"
        app.apply_settings(new)
        assert settings.load().colormap == "gray"
        print("OK", len(dialogs), len(listed))
        app.quit_app()
    root.after(300, check)
    root.mainloop()
if __name__ == "__main__":
    run()
""")
    env = dict(os.environ, REALHIRES_SETTINGS=str(tmp_path / "gui-settings.json"))
    proc = subprocess.run([sys.executable, str(script)], capture_output=True, text=True,
                          timeout=180, env=env)
    assert proc.returncode == 0, proc.stderr
    assert proc.stdout.startswith("OK 1 ")  # one summary pop-up was shown
