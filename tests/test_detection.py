"""End-to-end tests on synthetic real and fake hi-res files."""

import json
import shutil
from pathlib import Path

import numpy as np
import pytest

from realhires.analysis import BitDepthAccumulator
from realhires.cli import main
from realhires.verdict import FAKE, LOSSY, NOT_HIRES, REAL, SUSPECT, analyze_file

pytestmark = pytest.mark.skipif(shutil.which("ffmpeg") is None, reason="ffmpeg not installed")


@pytest.mark.parametrize("name", ["real_24_96", "real_24_192", "real_24_48"])
def test_real_hires(library, name):
    r = analyze_file(library[name])
    assert r.verdict == REAL, r.reasons
    assert r.effective_bits == 24
    assert r.upsampled_from is None


@pytest.mark.parametrize("name,source", [
    ("fake_up_44_to_96", 44100),
    ("fake_up_48_to_96", 48000),
    ("fake_up_96_to_192", 96000),
])
def test_upsampled_detected_with_source_rate(library, name, source):
    r = analyze_file(library[name])
    assert r.verdict == FAKE, r.reasons
    assert r.upsampled_from == source
    assert r.true_rate == source


def test_upsampled_with_soft_resampler_still_fake(library):
    # ffmpeg's default resampler has a soft transition, so 44.1 vs 48 kHz is
    # ambiguous, but the file must still be flagged as an upsample of one of them.
    r = analyze_file(library["fake_up_44_to_192"])
    assert r.verdict == FAKE
    assert r.upsampled_from in (44100, 48000)


@pytest.mark.parametrize("name", ["fake_padded_16_in_24", "fake_padded_16_in_float"])
def test_padded_bit_depth(library, name):
    r = analyze_file(library[name])
    assert r.verdict == FAKE, r.reasons
    assert r.effective_bits == 16
    assert r.upsampled_from is None  # spectrum is genuinely full-band
    assert r.estimated.startswith("16/")


def test_lossy_sourced(library):
    r = analyze_file(library["fake_from_mp3"])
    assert r.verdict == FAKE
    assert r.lossy_cutoff_hz and 14000 < r.lossy_cutoff_hz < 20300
    assert r.estimated.startswith("lossy")


def test_lossy_format(library):
    assert analyze_file(library["lossy_mp3"]).verdict == LOSSY


def test_cd_quality_is_not_hires(library):
    r = analyze_file(library["cd_16_44"])
    assert r.verdict == NOT_HIRES
    assert r.effective_bits == 16
    assert r.bandwidth_hz > 20000


def test_dull_recording_is_suspicious_not_fake(library):
    r = analyze_file(library["dull_96"])
    assert r.verdict == SUSPECT, r.reasons


def test_silence(library):
    r = analyze_file(library["silence"])
    assert r.verdict == SUSPECT
    assert "silence" in r.reasons[0]


def test_bit_accumulator_counts_lsb():
    acc = BitDepthAccumulator()
    rng = np.random.default_rng(1)
    acc.update((rng.integers(-2 ** 15, 2 ** 15, 10000, dtype=np.int64) << 16).astype(np.int32))
    assert acc.effective_bits() == 16
    acc = BitDepthAccumulator()
    acc.update((rng.integers(-2 ** 23, 2 ** 23, 10000, dtype=np.int64) << 8).astype(np.int32))
    assert acc.effective_bits() == 24


def test_cli_scan_with_exports(library, tmp_path, capsys):
    lib_dir = str(Path(library["real_24_96"]).parent)
    out_json, out_csv, plots = tmp_path / "r.json", tmp_path / "r.csv", tmp_path / "plots"
    rc = main([lib_dir, "-j", "2", "--json", str(out_json), "--csv", str(out_csv),
               "--plot", str(plots), "--no-color"])
    assert rc == 0
    out = capsys.readouterr().out
    assert "FAKE HI-RES" in out and "REAL HI-RES" in out
    data = json.loads(out_json.read_text())
    assert {d["verdict"] for d in data} >= {FAKE, REAL, NOT_HIRES, LOSSY}
    assert out_csv.read_text().startswith("path,verdict")
    assert len(list(plots.glob("*.svg"))) >= 10
