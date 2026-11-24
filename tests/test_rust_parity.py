"""The Rust version must judge every file exactly as the Python version does.

Runs the Rust CLI (`rust/target/release/realhires`, or $REALHIRES_RUST_CLI) on
the synthetic library and compares its JSON export with `analyze_file()`:
verdicts, flags, estimated resolution and every reason and summary must be
identical, and the measurements must agree to within rounding.  Skipped when
the Rust CLI has not been built (`cargo build --release` in `rust/`).
"""

import json
import math
import os
import shutil
import sqlite3
import subprocess
from pathlib import Path

import pytest

from realhires import settings as settings_mod
from realhires.fix import plan_fix
from realhires.store import ResultStore, settings_key
from realhires.verdict import analyze_file

ROOT = Path(__file__).resolve().parent.parent
EXE = ".exe" if os.name == "nt" else ""


def rust_cli():
    env = os.environ.get("REALHIRES_RUST_CLI")
    if env:
        return env
    for profile in ("release", "debug"):
        p = ROOT / "rust" / "target" / profile / f"realhires{EXE}"
        if p.is_file():
            return str(p)
    return None


pytestmark = [
    pytest.mark.skipif(shutil.which("ffmpeg") is None, reason="ffmpeg not installed"),
    pytest.mark.skipif(rust_cli() is None, reason="Rust CLI not built (cargo build --release in rust/)"),
]

EXACT = ["verdict", "summary", "reasons", "notes", "flags", "tags", "codec", "sample_rate", "channels",
         "claimed_bits", "is_float", "effective_bits", "true_rate", "upsampled_from", "grid_bits",
         "claimed", "estimated_real"]
CLOSE = {"bandwidth_hz": 0.01, "cutoff_drop_db": 1e-6, "noise_floor_db": 1e-6, "upsample_edge_hz": 0.01,
         "upsample_mid_hz": 0.01, "lossy_cutoff_hz": 0.01, "converter_floor_db": 1e-6, "grid_step": 0.011,
         "grid_gain_db": 0.011, "duration": 1e-9, "analysed_seconds": 1e-9}


def run_rust(*args, env=None):
    proc = subprocess.run([rust_cli(), *args], capture_output=True, text=True, encoding="utf-8", env=env)
    assert proc.returncode == 0, proc.stdout + proc.stderr
    return proc


def same_number(a, b, tol):
    if a is None or b is None:
        return a is None and b is None
    if isinstance(a, float) and math.isnan(a):
        return isinstance(b, float) and math.isnan(b)
    return abs(a - b) <= tol * max(1.0, abs(a))


def compare(py: dict, rs: dict, name: str):
    for k in EXACT:
        assert py[k] == rs[k], f"{name}: {k} differs\n  python: {py[k]!r}\n  rust:   {rs[k]!r}"
    for k, tol in CLOSE.items():
        assert same_number(py[k], rs[k], tol), f"{name}: {k} differs: python {py[k]!r}, rust {rs[k]!r}"
    for k in ("cliffs_db", "deep_cliffs_db"):
        assert py[k].keys() == rs[k].keys(), f"{name}: {k} rates differ"
        for rate in py[k]:
            assert same_number(py[k][rate], rs[k][rate], 0.11), f"{name}: {k}[{rate}] differs"
    assert py["bit_usage"].keys() == rs["bit_usage"].keys()
    for b in py["bit_usage"]:
        assert abs(py["bit_usage"][b] - rs["bit_usage"][b]) <= 1e-4


@pytest.fixture(scope="module")
def rust_results(library, tmp_path_factory):
    folder = Path(library["real_24_96"]).parent
    out = tmp_path_factory.mktemp("rust") / "r.json"
    run_rust(str(folder), "--no-cache", "--no-settings", "-j", "4", "--json", str(out), "--no-color")
    return {Path(d["path"]).name: d for d in json.loads(out.read_text(encoding="utf-8"))}


def test_every_file_gets_the_same_verdict_and_texts(library, rust_results):
    for name, path in library.items():
        py = json.loads(json.dumps(analyze_file(path).to_dict()))  # JSON round trip: str keys, None for NaN
        rs = rust_results[Path(path).name]
        compare(py, rs, name)


def test_fix_plans_agree(library, rust_results):
    cfg = settings_mod.Settings()
    for name, path in library.items():
        py = analyze_file(path)
        plan, why = plan_fix(py, cfg)
        proc = run_rust(path, "--no-cache", "--no-settings", "--fix", "--dry-run", "--no-color")
        if plan is None:
            assert "nothing to convert" in proc.stdout, name
        else:
            assert plan.description in proc.stdout, f"{name}: {plan.description!r} not in\n{proc.stdout}"


def test_saved_results_are_shared_both_ways(library, tmp_path):
    db = tmp_path / "shared.sqlite"
    env = dict(os.environ, REALHIRES_RESULTS=str(db))
    a, b = library["fake_up_44_to_96"], library["real_24_96"]
    # Python saves a result, Rust reuses it without analysing...
    with ResultStore(str(db)) as st:
        assert st.put(analyze_file(a), settings_key())
    out = run_rust(a, "--no-settings", "--no-color", env=env).stdout
    assert "1 file(s) unchanged since an earlier scan were not analysed again, 0 analysed" in out
    # ... and Rust saves one that Python then finds.
    run_rust(b, "--no-settings", "--no-color", env=env)
    with ResultStore(str(db)) as st:
        r = st.lookup(b, settings_key())
    assert r is not None and r.verdict == analyze_file(b).verdict
    assert r.saved["version"] and not r.saved["outdated"]
    rows = sqlite3.connect(str(db)).execute("SELECT count(*) FROM results").fetchone()[0]
    assert rows == 2


def test_settings_key_and_settings_file_are_shared(tmp_path):
    cfg = settings_mod.Settings()
    cfg.cliff_db = 31.5
    cfg.shown_verdicts = ["ERROR"]
    path = tmp_path / "settings.json"
    settings_mod.save(cfg, str(path))
    out = run_rust("--settings", str(path), "--list-settings").stdout
    assert "cliff_db               = 31.5" in out
    assert "shown_verdicts         = ['ERROR']" in out


def test_spectrogram_png_and_csv(library, tmp_path):
    out = tmp_path / "png"
    csv_path = tmp_path / "r.csv"
    run_rust(library["fake_up_44_to_96"], "--no-cache", "--spectrogram", str(out), "--csv", str(csv_path),
             "--plot", str(tmp_path / "svg"))
    pngs = list(out.glob("*.png"))
    assert len(pngs) == 1 and pngs[0].read_bytes().startswith(b"\x89PNG")
    assert csv_path.read_text(encoding="utf-8").startswith("path,verdict,summary")
    assert len(list((tmp_path / "svg").glob("*.svg"))) == 1


# ------------------------------------------------------------------ fixing ---
def _pcm(path, channels=2):
    import numpy as np
    from realhires.probe import iter_pcm

    return np.concatenate(list(iter_pcm(str(path), channels)))


def _title(path):
    return subprocess.run(["ffprobe", "-v", "error", "-show_entries", "format_tags=title,artist", "-of", "json",
                           str(path)], capture_output=True, text=True, encoding="utf-8").stdout


def test_rust_fix_in_place_moves_original_to_backup_folder(library, tmp_path):
    from realhires.cli import collect
    from realhires.fix import BACKUP_DIR_NAME
    from realhires.probe import probe

    f = tmp_path / "Album" / "01 Song.flac"
    f.parent.mkdir()
    shutil.copy(library["fake_up_44_to_96"], f)
    out = run_rust(str(tmp_path), "--no-cache", "--no-settings", "--fix", "--no-color").stdout
    moved = "original moved to " + os.path.join(BACKUP_DIR_NAME, "01 Song.flac")
    assert "Fix: 1 fixed, 0 failed." in out and moved in out, out
    info = probe(str(f))
    assert (info.sample_rate, info.claimed_bits) == (44100, 16)
    backup = f.parent / BACKUP_DIR_NAME / "01 Song.flac"
    assert backup.exists() and probe(str(backup)).sample_rate == 96000
    assert collect([str(tmp_path)], recursive=True) == [str(f)]
    assert analyze_file(str(f)).verdict == "NOT HI-RES"
    again = run_rust(str(tmp_path), "--no-cache", "--no-settings", "--no-color").stdout
    assert "Scanned 1 file(s): 1 not hi-res" in again  # the backup folder is never scanned


def test_rust_fix_keeps_bak_and_tags(library, tmp_path):
    from realhires.probe import probe

    src = tmp_path / "Track テスト.flac"
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", library["fake_up_96_to_192"], "-c:a", "flac",
                    "-sample_fmt", "s32", "-metadata", "title=曲名", "-metadata", "artist=A", str(src)], check=True)
    out = run_rust(str(src), "--no-cache", "--no-settings", "--fix", "--fix-backup", "bak").stdout
    assert "24/192 → 24/96" in out and "Fix: 1 fixed" in out, out
    assert (tmp_path / "Track テスト.flac.bak").exists()
    info = probe(str(src))
    assert info.sample_rate == 96000 and info.claimed_bits == 24
    tags = _title(src)
    assert "曲名" in tags and '"artist": "A"' in tags


def test_rust_fix_restores_16_bit_master_bit_exactly(library, tmp_path):
    import numpy as np

    master = Path(library["fake_up_48_to_96_swr"]).parent / "m48.wav"
    src = tmp_path / "src"
    src.mkdir()
    for rate in (96000, 192000):
        subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", str(master), "-ar", str(rate), "-c:a", "flac",
                        "-sample_fmt", "s32", "-metadata", "title=曲名", str(src / f"fake_{rate}.flac")],
                       check=True)
    out_dir = tmp_path / "out"
    out = run_rust(str(src), "--no-cache", "--no-settings", "--fix", "--fix-dir", str(out_dir)).stdout
    assert out.count("restored bit-exactly") == 2, out
    want = _pcm(master)
    outs = sorted(out_dir.iterdir())
    assert len(outs) == 2
    for f in outs:
        got = _pcm(f)
        assert got.shape == want.shape and np.array_equal(got, want), f
        assert "曲名" in _title(f)


def test_rust_fix_keeps_gapless_joins(tmp_path):
    import numpy as np
    from conftest import ffmpeg_write, hq_resample, music_like
    from realhires.fix import _resample_filter, plan_fix

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
    out_dir = tmp_path / "out"
    out = run_rust(str(d), "--no-cache", "--no-settings", "--set", "fix_bits=As detected", "--fix", "--fix-dir",
                   str(out_dir)).stdout
    assert "2 gapless join(s) found" in out and out.count("joins stay gapless.") == 3, out
    joined = np.concatenate([_pcm(f) for f in sorted(out_dir.iterdir())]).astype(np.float64)
    cfg = settings_mod.Settings()
    cfg.fix_bits = "As detected"
    plan, _ = plan_fix(analyze_file(str(d / "01 track.wav")), cfg)
    whole = tmp_path / "whole.wav"
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", str(up), "-af", _resample_filter(plan),
                    "-c:a", "pcm_s24le", str(whole)], check=True)
    ref = _pcm(whole).astype(np.float64)
    assert joined.shape == ref.shape
    err = np.abs(joined - ref).max(axis=1) / 2.0 ** 31
    assert err.max() < 10 ** (-100 / 20), 20 * np.log10(err.max())


# ------------------------------------------------------- texts and lyrics ---
def run_python(*args, env=None):
    import sys

    proc = subprocess.run([sys.executable, "-m", "realhires", *args], capture_output=True, text=True,
                          encoding="utf-8", env=env, cwd=str(ROOT))
    return proc


def test_verbose_output_is_the_same(library):
    """Every line of `-v`, including what Fix does with each fake file and whether it stays fake."""
    folder = str(Path(library["real_24_96"]).parent)
    args = (folder, "-v", "--no-cache", "--no-settings", "--no-color", "-j", "4")
    py = run_python(*args)
    rs = subprocess.run([rust_cli(), *args], capture_output=True, text=True, encoding="utf-8")
    assert py.returncode == rs.returncode == 0, py.stderr + rs.stderr
    assert "· Fix to real resolution" in py.stdout
    for a, b in zip(py.stdout.splitlines(), rs.stdout.splitlines()):
        assert a == b, f"python: {a!r}\nrust:   {b!r}"
    assert len(py.stdout.splitlines()) == len(rs.stdout.splitlines())


def test_lyrics_downloads_match_python(tmp_path, monkeypatch):
    """The same requests, messages and files as `realhires --lyrics-only`, against a fake LRCLIB."""
    from test_lyrics import FakeLrclib, make_album, rec

    fake = FakeLrclib()
    try:
        env = dict(os.environ, REALHIRES_LYRICS_INTERVAL="0", NO_PROXY="127.0.0.1,localhost",
                   no_proxy="127.0.0.1,localhost")
        album = {}
        for who in ("python", "rust"):
            fake.songs.clear()
            album[who] = tmp_path / who / "album"
            album[who].parent.mkdir()
            make_album(album[who], fake)
        fake.songs.append(rec("Alpha", artist="The Band;Guest", synced="[00:00.30] with a guest", rid=9))
        server = f"lyrics_server={fake.url}"
        runs = [("--lyrics-only", "--set", server), ("--lyrics-only", "--set", server, "-v"),
                ("--lyrics-only", "--lyrics-existing", "replace", "--dry-run", "--set", server),
                ("--lyrics-only", "--lyrics-existing", "skip-any", "--set", server, "-v")]
        for args in runs:
            out, requests = {}, {}
            for who in ("python", "rust"):
                fake.requests.clear()
                cmd = (str(album[who]), "--no-settings", "--no-color", *args)
                proc = run_python(*cmd, env=env) if who == "python" else subprocess.run(
                    [rust_cli(), *cmd], capture_output=True, text=True, encoding="utf-8", env=env)
                assert proc.returncode == 0, proc.stdout + proc.stderr
                out[who] = proc.stdout.replace(str(album[who]), "ALBUM")
                requests[who] = list(fake.requests)
            assert out["python"] == out["rust"], args
            assert requests["python"] == requests["rust"], args
        for f in sorted(os.listdir(album["python"])):
            assert (album["rust"] / f).exists(), f
            if f.endswith((".lrc", ".txt")):
                assert (album["python"] / f).read_bytes() == (album["rust"] / f).read_bytes(), f
        assert sorted(os.listdir(album["python"])) == sorted(os.listdir(album["rust"]))
        # failures and the server check
        fake.answers = [(400, {})]
        proc = subprocess.run([rust_cli(), str(album["rust"] / "05 Epsilon.flac"), "--no-settings", "--lyrics-only",
                               "--set", server], capture_output=True, text=True, encoding="utf-8", env=env)
        assert proc.returncode == 1 and "refused the request (HTTP 400: fake)" in proc.stdout, proc.stdout
        assert "Lyrics: 1 failed." in proc.stdout
        proc = subprocess.run([rust_cli(), "--check-lyrics-server", "--set", server], capture_output=True, text=True,
                              encoding="utf-8", env=env)
        assert proc.returncode == 0 and "can be reached" in proc.stdout, proc.stdout + proc.stderr
        fake.answers = [(400, {})]
        proc = subprocess.run([rust_cli(), "--check-lyrics-server", "--set", server], capture_output=True, text=True,
                              encoding="utf-8", env=env)
        assert proc.returncode == 1 and "error:" in proc.stderr
    finally:
        fake.server.shutdown()


def test_lyrics_after_a_scan_use_the_scan_tags(tmp_path):
    from test_lyrics import FakeLrclib, rec, tagged

    fake = FakeLrclib()
    try:
        env = dict(os.environ, REALHIRES_LYRICS_INTERVAL="0", NO_PROXY="127.0.0.1,localhost",
                   no_proxy="127.0.0.1,localhost")
        song = tagged(tmp_path / "Eta.flac", title="Eta", artist="The Band", album="First")
        fake.songs.append(rec("Eta", synced="[00:00.20] Eta sings", rid=6))
        proc = subprocess.run([rust_cli(), song, "--no-cache", "--no-settings", "--lyrics", "--set",
                               f"lyrics_server={fake.url}"], capture_output=True, text=True, encoding="utf-8", env=env)
        assert proc.returncode == 0 and "NOT HI-RES" in proc.stdout and "Lyrics: 1 with synced lyrics." in proc.stdout
        assert (tmp_path / "Eta.lrc").read_text(encoding="utf-8") == "[00:00.20] Eta sings\n"
    finally:
        fake.server.shutdown()


@pytest.mark.parametrize("mode", [0, 1, 2, 3])
def test_lyrics_save_modes_write_the_same_files(tmp_path, mode):
    """2.0: lyrics saved as .lrc, inside the audio file (FLAC/MP3, as LRCGET), both, or plain inside:
    both versions write byte-identical audio and lyrics files and print the same messages."""
    from realhires.settings import LYRICS_SAVE_CHOICES
    from test_lyrics import FakeLrclib, rec, tagged

    have_mp3 = " libmp3lame " in subprocess.run(["ffmpeg", "-hide_banner", "-encoders"], capture_output=True,
                                                 text=True).stdout
    fake = FakeLrclib()
    try:
        env = dict(os.environ, REALHIRES_LYRICS_INTERVAL="0", NO_PROXY="127.0.0.1,localhost",
                   no_proxy="127.0.0.1,localhost")
        base = tmp_path / "base"
        base.mkdir()
        band = dict(artist="The Band", album="First")
        tagged(base / "01 Alpha.flac", title="Alpha", **band)
        tagged(base / "03 Gamma.flac", title="Gamma", **band)
        tagged(base / "04 Delta.flac", title="Delta", **band)
        subprocess.run(["ffmpeg", "-v", "error", "-f", "lavfi", "-i", "sine=duration=4", "-metadata", "title=Epsilon",
                        "-metadata", "artist=The Band", "-metadata", "album=First", str(base / "05 Epsilon.wav")],
                       check=True)
        if have_mp3:
            for major in (3, 4):
                subprocess.run(["ffmpeg", "-v", "error", "-f", "lavfi", "-i", "sine=duration=4", "-metadata",
                                f"title=Beta{major}", "-metadata", "artist=The Band", "-metadata", "album=First",
                                "-c:a", "libmp3lame", "-id3v2_version", str(major),
                                str(base / f"02 Beta{major}.mp3")], check=True)
        fake.songs += [rec("Alpha", synced="[00:00.50] First line ♪\n[00:02.00] 二行目", rid=1),
                       rec("Beta3", synced="[00:01.00] Beta three", rid=2),
                       rec("Beta4", synced="[00:01.00] Beta four", rid=3),
                       rec("Gamma", plain="Gamma has no timing", rid=4),
                       rec("Delta", instrumental=True, rid=5),
                       rec("Epsilon", synced="[00:00.10] Wave file", rid=6)]
        server = f"lyrics_server={fake.url}"
        choice = f"lyrics_save_as={LYRICS_SAVE_CHOICES[mode]}"
        out = {}
        for who in ("python", "rust"):
            folder = tmp_path / who
            shutil.copytree(base, folder)
            args = (str(folder), "--no-settings", "--no-color", "--lyrics-only", "--set", server, "--set", choice, "-v")
            for run in (1, 2):  # the second run must skip every song that got lyrics
                proc = run_python(*args, env=env) if who == "python" else subprocess.run(
                    [rust_cli(), *args], capture_output=True, text=True, encoding="utf-8", env=env)
                assert proc.returncode == 0, proc.stdout + proc.stderr
                out[(who, run)] = proc.stdout.replace(str(folder), "ALBUM")
        assert out[("python", 1)] == out[("rust", 1)]
        assert out[("python", 2)] == out[("rust", 2)]
        assert "skipped (already had lyrics)" in out[("python", 2)]
        names = sorted(os.listdir(tmp_path / "python"))
        assert names == sorted(os.listdir(tmp_path / "rust"))
        for n in names:
            a, b = (tmp_path / "python" / n).read_bytes(), (tmp_path / "rust" / n).read_bytes()
            assert a == b, f"{n} differs ({LYRICS_SAVE_CHOICES[mode]})"
        if mode in (1, 3):  # embedded only: no .lrc next to the FLAC and MP3 songs, but the WAV gets one
            assert not [n for n in names if n.endswith(".lrc") and not n.startswith("05")]
            assert "05 Epsilon.lrc" in names
    finally:
        fake.server.shutdown()
