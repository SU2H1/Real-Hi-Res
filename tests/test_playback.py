"""The 2.0 playback engine (realhires/player.py), with a hand-pulled output and
miniaudio's NULL backend, so no sound hardware is needed."""

import shutil
import subprocess
import time

import numpy as np
import pytest

from realhires import eq, player
from realhires.player import ManualOutput, Player

needs_ffmpeg = pytest.mark.skipif(shutil.which("ffmpeg") is None, reason="ffmpeg not installed")
pytestmark = needs_ffmpeg


def song(path, seconds=1.0, rate=44100, channels=2, seed=1, codec=("-c:a", "flac"), **tags):
    """Noise at -12 dBFS (different in every channel), encoded with `codec`."""
    rng = np.random.default_rng(seed)
    x = rng.uniform(-0.25, 0.25, (int(rate * seconds), channels))
    cmd = ["ffmpeg", "-v", "error", "-y", "-f", "f64le", "-ar", str(rate), "-ac", str(channels), "-i", "-"]
    for k, v in tags.items():
        cmd += ["-metadata", f"{k}={v}"]
    subprocess.run(cmd + [*codec, str(path)], input=x.astype("<f8").tobytes(), check=True)
    return str(path)


def decoded(path, fmt="f32le", dtype="<f4", extra=()):
    out = subprocess.run(["ffmpeg", "-v", "error", "-i", str(path), *extra, "-f", fmt, "-ac", "2", "-"],
                         capture_output=True, check=True).stdout
    return np.frombuffer(out, dtype).reshape(-1, 2)


def pull_all(p, out, block=1000, limit=60.0):
    """Pull frames until the engine says the queue is finished; (frames, events)."""
    got, events, end = [], [], time.monotonic() + limit
    while time.monotonic() < end:
        got.append(out.pull(block))
        events += p.poll()
        if any(e[0] in ("finished", "error") for e in events):
            break
    return np.concatenate(got) if got else np.zeros((0, 2), np.float32), events


def wait_for(cond, timeout=10.0):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if cond():
            return True
        time.sleep(0.01)
    return False


def test_plays_exactly_the_decoded_samples_and_counts_what_is_heard(tmp_path):
    a = song(tmp_path / "a.flac", 1.0)
    out = ManualOutput()
    p = Player(out)
    p.set_volume(1.0)
    p.play(a)
    first = out.pull(4410)
    assert out.opened == [44100], "the device must open at the song's rate"
    ref = decoded(a)
    assert np.array_equal(first, ref[:4410])
    assert abs(p.position() - 0.1) < 1e-9 and p.playing and p.current == a
    # volume: gain = volume squared, applied after decoding
    p.set_volume(0.5)
    assert np.allclose(out.pull(1000), ref[4410:5410] * 0.25, atol=1e-7)
    p.set_volume(0.5, muted=True)
    assert not np.any(out.pull(1000)) and player.fmt_gain(p.gain, True) == "Muted"
    p.set_volume(1.0)
    # pause: nothing is handed over and the position stays
    p.pause()
    assert len(out.pull(1000)) == 0 and abs(p.position() - 6410 / 44100) < 1e-9
    p.resume()
    assert np.array_equal(out.pull(500), ref[6410:6910])
    # seek: FFmpeg restarts there, the old buffer is dropped
    p.seek(0.5)
    assert p.position() == pytest.approx(0.5)
    after = out.pull(2000)
    assert np.allclose(after, ref[22050:24050], atol=1e-6)
    assert p.position() == pytest.approx(0.5 + 2000 / 44100)
    rest, events = pull_all(p, out)
    assert events[-1] == ("finished", a) and not p.active
    assert p.position() == pytest.approx(1.0, abs=1e-6)
    p.stop()
    assert p.current is None
    p.close()


def test_gapless_songs_make_one_continuous_stream(tmp_path):
    a = song(tmp_path / "a.flac", 1.3, seed=1)
    b = song(tmp_path / "b.flac", 0.7, seed=2)
    out = ManualOutput()
    p = Player(out)
    p.set_volume(1.0)
    p.play(a)
    p.set_next(b)
    got, events = pull_all(p, out, block=777)
    ref = np.concatenate([decoded(a), decoded(b)])
    assert len(got) == len(ref) and np.array_equal(got, ref), "a gap or a lost sample between the songs"
    assert events == [("advanced", b), ("finished", b)]
    assert out.opened == [44100]  # one device for both
    p.close()


def test_repeat_song_loops_without_a_gap_and_gapless_off_stops(tmp_path):
    a = song(tmp_path / "a.flac", 0.4)
    out = ManualOutput()
    p = Player(out)
    p.set_volume(1.0)
    p.play(a)
    p.set_next(a)  # repeat song: the app sets the same song again whenever it starts
    got, starts = [], 0
    while sum(len(f) for f in got) < 3 * 17640:
        got.append(out.pull(500))
        for e in p.poll():
            assert e == ("advanced", a)
            starts += 1
            if starts < 2:
                p.set_next(a)
    got = np.concatenate(got)
    assert np.array_equal(got[:3 * 17640], np.tile(decoded(a), (3, 1))), "a gap between the repeats"
    assert starts == 2
    p.set_gapless(False)
    p.set_next(a)
    p.play(a)
    got, events = pull_all(p, out)
    assert len(got) == 17640 and events == [("finished", a)], "gapless off still chained the next song"
    p.close()


def test_next_song_at_another_rate_reopens_the_device(tmp_path):
    a = song(tmp_path / "a.flac", 0.5, rate=44100)
    b = song(tmp_path / "b.flac", 0.5, rate=48000, seed=3)
    out = ManualOutput()  # runs at any rate
    p = Player(out)
    p.set_volume(1.0)
    p.play(a)
    p.set_next(b)
    got, events = pull_all(p, out)
    assert out.opened == [44100, 48000]
    assert np.array_equal(got, np.concatenate([decoded(a), decoded(b)]))
    assert events == [("advanced", b), ("finished", b)]
    p.close()


def test_resamples_with_soxr_when_the_device_cannot_run_at_the_songs_rate(tmp_path):
    a = song(tmp_path / "a.flac", 1.0, rate=96000)
    out = ManualOutput(rates=[48000])
    p = Player(out)
    p.set_volume(1.0)
    p.play(a)
    got, events = pull_all(p, out)
    assert out.opened == [48000] and abs(len(got) - 48000) <= 64
    info = p.output_info()
    assert info["rate"] == 0 or info["rate"] == 48000
    want = "soxr" if player.has_soxr() else "FFmpeg's default resampler"
    assert player.resampling_label(96000, 48000, player.resampler_name()) == f"From 96 kHz to 48 kHz ({want})"
    assert player.resampling_label(96000, 96000, want) == "None: the device runs at the song's rate"
    assert player.output_label(48000) == "48 kHz, 32-bit float, stereo"
    p.close()


def test_mono_is_played_on_both_channels_at_full_level(tmp_path):
    m = song(tmp_path / "m.flac", 0.3, channels=1)
    out = ManualOutput()
    p = Player(out)
    p.set_volume(1.0)
    p.play(m)
    got, _ = pull_all(p, out)
    mono = subprocess.run(["ffmpeg", "-v", "error", "-i", m, "-f", "f32le", "-"], capture_output=True,
                          check=True).stdout
    mono = np.frombuffer(mono, "<f4")
    assert np.array_equal(got[:, 0], mono) and np.array_equal(got[:, 1], mono)
    p.close()


def test_equalizer_changes_the_samples_exactly_like_the_reference(tmp_path):
    a = song(tmp_path / "a.flac", 0.25)
    state = eq.apply_preset(eq.default_state(), "Rock")
    state["enabled"] = True
    state["parametric"] = {"enabled": True, "preamp": -2.0,
                           "bands": [{"type": "high_shelf", "freq": 6000.0, "gain": 3.0, "q": 0.8, "on": True}]}
    out = ManualOutput()
    p = Player(out)
    p.set_volume(1.0)
    p.set_eq(state)
    p.play(a)
    got, _ = pull_all(p, out)
    x = decoded(a, "f64le", "<f8")
    ref = x * 10 ** (eq.preamp_db(state) / 20)
    for c in eq.stages(state, 44100):
        bq = eq.Biquad(c, 2)
        ref = np.array([[bq.process(v, ch) for ch, v in enumerate(row)] for row in ref])
    assert len(got) == len(ref)
    assert np.max(np.abs(got - ref)) < 2e-6, "the EQ does not match the reference biquads"
    assert np.max(np.abs(got - x)) > 0.01, "the EQ did nothing"
    assert player.eq_summary(state).startswith("Graphic: Rock, parametric: 1 band (pre-amp −")
    assert player.eq_summary(eq.default_state()) == "Off"
    # turning it off while playing restarts FFmpeg at the position without the EQ
    p.play(a)
    out.pull(2205)
    gen = p._gen
    p.set_eq(eq.default_state())
    assert wait_for(lambda: p._eq_timer is None and p._gen > gen)
    assert p.position() == pytest.approx(0.05)
    clean = out.pull(2205)
    assert np.allclose(clean, x[2205:4410], atol=1e-6)
    p.close()


def test_unreadable_file_is_reported(tmp_path):
    bad = tmp_path / "broken.flac"
    bad.write_bytes(b"this is not audio" * 100)
    out = ManualOutput()
    p = Player(out)
    p.set_volume(1.0)
    p.play(str(bad))
    assert wait_for(lambda: p._events)
    (kind, path, message), = p.poll()
    assert kind == "error" and path == str(bad) and message.startswith("The file broken.flac ")
    assert message.endswith(".") and not p.active
    p.play(str(tmp_path / "gone.flac"))
    assert wait_for(lambda: p._events)
    assert p.poll()[0][2] == "The file gone.flac does not exist any more."
    p.close()


def test_a_failing_next_song_ends_the_stream_cleanly(tmp_path):
    a = song(tmp_path / "a.flac", 0.3)
    bad = tmp_path / "bad.flac"
    bad.write_bytes(b"\0" * 4000)
    out = ManualOutput()
    p = Player(out)
    p.set_volume(1.0)
    p.play(a)
    p.set_next(str(bad))
    got, events = pull_all(p, out)
    assert len(got) == 13230 and events == [("finished", a)]  # the app then tries bad.flac and reports it
    p.close()


def test_formatting_for_the_playback_info():
    assert player.fmt_gain(0.5) == "−6.0 dB" and player.fmt_gain(1.0) == "0.0 dB"
    assert player.fmt_khz(44100) == "44.1 kHz" and player.channels_label(6) == "6 channels"
    assert player.bit_depth_label(24, False, False) == "24-bit"
    assert player.bit_depth_label(32, True, False) == "32-bit float"
    assert player.bit_depth_label(0, False, True) == "—" and player.bit_rate_label(2304000) == "2,304 kbps"


def _null_backend():
    try:
        import miniaudio

        return miniaudio.Backend.NULL
    except ImportError:
        return None


@pytest.mark.skipif(_null_backend() is None, reason="miniaudio not installed")
def test_miniaudio_null_backend_plays_in_real_time(tmp_path):
    a = song(tmp_path / "a.flac", 0.8)
    out = player.MiniaudioOutput(backends=[_null_backend()])
    assert [d.name for d in out.devices()] == ["NULL Playback Device"]
    p = Player(out)
    p.set_volume(1.0)
    t0 = time.monotonic()
    p.play(a)
    assert wait_for(lambda: p.position() > 0.2, 5)
    assert p.output_info()["rate"] == 44100 and out.backend == "Null"
    events = []
    assert wait_for(lambda: events.extend(p.poll()) or events, 10)
    assert events == [("finished", a)] and time.monotonic() - t0 > 0.6, "not played in real time"
    p.close()
