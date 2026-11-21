"""Smoke-test a built release: run the frozen CLI on three generated WAV files.

Usage: python packaging/smoke_test.py PATH/TO/realhires-cli[.exe]

The CLI is run with an empty PATH so it must use its bundled FFmpeg, and it is
run twice: the second run must reuse the saved results of the first, which
proves that SQLite is bundled too.
"""

import json
import os
import subprocess
import sys
import tempfile
import wave

import numpy as np


def write_wav(path: str, samples: np.ndarray, rate: int, width: int) -> None:
    """samples: int64 array (frames, 2) already scaled to `width` bytes."""
    raw = samples.astype("<i4").tobytes()
    if width == 3:  # keep the low 3 bytes of each little-endian int32
        raw = np.frombuffer(raw, dtype=np.uint8).reshape(-1, 4)[:, :3].tobytes()
    elif width == 2:
        raw = samples.astype("<i2").tobytes()
    with wave.open(path, "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(width)
        w.setframerate(rate)
        w.writeframes(raw)


def main() -> int:
    for stream in (sys.stdout, sys.stderr):  # CI consoles may be cp1252
        stream.reconfigure(encoding="utf-8", errors="replace")
    cli = os.path.abspath(sys.argv[1])
    rng = np.random.default_rng(0)
    tmp = tempfile.mkdtemp()
    noise = rng.normal(scale=0.05, size=(96000 * 3, 2)).clip(-1, 1)
    # 16-bit audio padded into a 24-bit/96 kHz WAV  -> FAKE HI-RES
    s16 = np.round(noise * 32767).astype(np.int64)
    write_wav(os.path.join(tmp, "padded_24_96.wav"), s16 << 8, 96000, 3)
    # Genuine 24-bit white noise at 96 kHz            -> REAL HI-RES
    write_wav(os.path.join(tmp, "real_24_96.wav"), np.round(noise * 8388607).astype(np.int64), 96000, 3)
    # CD quality                                       -> NOT HI-RES
    write_wav(os.path.join(tmp, "cd_16_44 テスト曲 é.wav"), s16[: 44100 * 3], 44100, 2)

    out_json = os.path.join(tmp, "r.json")
    env = dict(os.environ, PATH="")
    env.pop("PYTHONIOENCODING", None)
    env.pop("PYTHONUTF8", None)
    # saved results (SQLite) go to a temporary database, not the runner's home
    env["REALHIRES_RESULTS"] = os.path.join(tmp, "saved", "results.sqlite")
    if os.name == "nt":
        env["PATH"] = os.environ.get("SystemRoot", r"C:\Windows") + r"\System32"
    want = {"padded_24_96.wav": "FAKE HI-RES", "real_24_96.wav": "REAL HI-RES", "cd_16_44 テスト曲 é.wav": "NOT HI-RES"}
    # The second run must reuse the saved results of the first one.
    for run, expect in (("first", "0 file(s) unchanged"), ("second", "3 file(s) unchanged")):
        proc = subprocess.run([cli, tmp, "--no-color", "-j", "2", "--json", out_json],
                              capture_output=True, text=True, encoding="utf-8", errors="replace",
                              env=env, timeout=600)
        print(proc.stdout)
        print(proc.stderr, file=sys.stderr)
        if proc.returncode != 0:
            print(f"CLI exited with {proc.returncode} ({run} run)", file=sys.stderr)
            return 1
        got = {os.path.basename(r["path"]): r["verdict"] for r in json.load(open(out_json, encoding="utf-8"))}
        if got != want:
            print(f"Unexpected verdicts ({run} run): {got}", file=sys.stderr)
            return 1
        if expect not in proc.stdout:
            print(f"Saved results were not used as expected ({run} run): no '{expect}'", file=sys.stderr)
            return 1
    print("Smoke test passed (saved results reused on the second run):", got)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
