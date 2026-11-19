"""End-to-end tests on synthetic real and fake hi-res files."""

import json
import shutil
from pathlib import Path

import numpy as np
import pytest

from realhires.analysis import BitDepthAccumulator, GridAccumulator
from realhires.cli import main
from realhires.verdict import FAKE, LOSSY, NOT_HIRES, REAL, SUSPECT, analyze_file

pytestmark = pytest.mark.skipif(shutil.which("ffmpeg") is None, reason="ffmpeg not installed")


@pytest.mark.parametrize("name", ["real_24_96", "real_24_192", "real_24_48"])
def test_real_hires(library, name):
    r = analyze_file(library[name])
    assert r.verdict == REAL, r.reasons
    assert r.effective_bits == 24
    assert r.upsampled_from is None
    assert r.grid_step is None


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


def test_upsampled_with_soft_resampler_finds_source_rate(library):
    # ffmpeg's default resampler has a soft transition that reaches past 24 kHz;
    # its -6 dB point still sits on the 22.05 kHz Nyquist of the 44.1 kHz source.
    r = analyze_file(library["fake_up_44_to_192"])
    assert r.verdict == FAKE, r.reasons
    assert r.upsampled_from == 44100
    assert r.bandwidth_hz <= 22050  # resampler residue above the cliff is not content


def test_soft_resampler_48_to_96_is_caught(library):
    # Its narrow cliff is only ~18 dB; the deep cliff (and the 16-bit grid that a
    # 2x upsample keeps in every second sample) give it away.
    r = analyze_file(library["fake_up_48_to_96_swr"])
    assert r.verdict == FAKE, r.reasons
    assert r.upsampled_from == 48000
    assert r.estimated == "16/48"


@pytest.mark.parametrize("name", ["fake_padded_16_in_24", "fake_padded_16_in_float"])
def test_padded_bit_depth(library, name):
    r = analyze_file(library[name])
    assert r.verdict == FAKE, r.reasons
    assert r.effective_bits == 16
    assert r.upsampled_from is None  # spectrum is genuinely full-band
    assert r.estimated.startswith("16/")


@pytest.mark.parametrize("name,gain", [
    ("fake_gain_16_to_24", -3.0),
    ("fake_dither_16_to_24", 0.0),
    ("fake_gain_16_to_float", -1.5),
])
def test_gain_or_dither_on_16_bit_master(library, name, gain):
    # The low bits are full (so the lowest-set-bit test sees 24 bits), but every
    # sample sits on the grid of the 16-bit master's steps.
    r = analyze_file(library[name])
    assert r.verdict == FAKE, r.reasons
    assert r.grid_bits == 16 and r.effective_bits == 16
    assert abs(r.grid_gain_db - gain) < 0.1
    assert r.estimated == "16/44.1"
    assert "grid" in r.reasons[0]


def test_analog_transfer_with_converter_noise_is_real(library):
    r = analyze_file(library["analog_transfer_96"])
    assert r.verdict == REAL, r.reasons
    assert r.bandwidth_hz < 24500
    assert any("noise floor" in n for n in r.notes)


def test_dull_upsample_with_converter_noise_is_fake(library):
    # The 48 kHz master's own converter noise stops dead at 24 kHz.
    r = analyze_file(library["fake_dull_up_48_to_96"])
    assert r.verdict == FAKE, r.reasons
    assert r.upsampled_from == 48000


def test_grid_accumulator():
    rng = np.random.default_rng(4)
    k = np.clip(np.round(rng.laplace(0, 3000, (200000, 2))), -32768, 32767)
    on_grid = np.round(k * 256 * 10 ** (-2.5 / 20) + rng.random(k.shape) - rng.random(k.shape))
    acc = GridAccumulator(2)
    acc.update((on_grid.astype(np.int64) << 8).astype(np.int32))
    g = acc.result(int(np.abs(on_grid).max()) << 8)
    assert g is not None and g.bits == 16 and abs(g.gain_db + 2.5) < 0.05
    real = np.round(rng.laplace(0, 3000 * 256, (200000, 2)))
    acc = GridAccumulator(2)
    acc.update((np.clip(real, -2 ** 23, 2 ** 23 - 1).astype(np.int64) << 8).astype(np.int32))
    assert acc.result(2 ** 31 - 1) is None


def _need(library, name):
    if name not in library:
        pytest.skip("this FFmpeg build has no MP3 encoder")
    return library[name]


def test_lossy_sourced(library):
    r = analyze_file(_need(library, "fake_from_mp3"))
    assert r.verdict == FAKE
    assert r.lossy_cutoff_hz and 14000 < r.lossy_cutoff_hz < 20300
    assert r.estimated.startswith("lossy")


def test_lossy_format(library):
    assert analyze_file(_need(library, "lossy_mp3")).verdict == LOSSY


def test_cd_quality_is_not_hires(library):
    r = analyze_file(library["cd_16_44"])
    assert r.verdict == NOT_HIRES
    assert r.effective_bits == 16
    assert r.bandwidth_hz > 20000


def test_dull_recording_is_suspicious_not_fake(library):
    # No content above ~10 kHz and only rounding noise above that: nothing
    # proves either way whether it was recorded at 96 kHz.
    r = analyze_file(library["dull_96"])
    assert r.verdict == SUSPECT, r.reasons
    assert "rounding noise" in r.reasons[0]


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
    assert {d["verdict"] for d in data} >= {FAKE, REAL, NOT_HIRES}
    assert out_csv.read_text().startswith("path,verdict")
    assert len(list(plots.glob("*.svg"))) >= 10


def test_gui_plot_payload_is_compact(library):
    r = analyze_file(library["fake_up_44_to_96"], keep_plot=True)
    p = r.plot
    assert p and 100 < len(p["freqs"]) <= 900 and len(p["freqs"]) == len(p["db"])
    assert p["nyquist"] == 48000 and abs(p["bandwidth_hz"] - r.bandwidth_hz) < 1
    assert r.spectrum is None  # full arrays are not kept unless asked for
    assert "plot" not in r.to_dict()
