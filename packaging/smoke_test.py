"""Smoke-test a built release: run the frozen CLI on three generated WAV files.

Usage: python packaging/smoke_test.py PATH/TO/realhires-cli[.exe]

The CLI is run with an empty PATH so it must use its bundled FFmpeg, and it is
run twice: the second run must reuse the saved results of the first, which
proves that SQLite is bundled too.  Then it downloads lyrics for a tagged WAV
from a fake LRCLIB server on 127.0.0.1 (urllib, JSON, the .lrc file), and asks
lrclib.net itself once: a certificate that cannot be verified fails the test,
because a frozen OpenSSL may not find the system's certificates; a network
problem of the runner only prints a warning.
"""

import json
import os
import struct
import subprocess
import sys
import tempfile
import threading
import wave
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

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


def add_info_tags(path: str, **tags: str) -> None:
    """Append a RIFF LIST/INFO chunk (INAM = title, IART = artist) to a WAV file."""
    ids = {"title": b"INAM", "artist": b"IART", "album": b"IPRD"}
    body = b"INFO"
    for key, value in tags.items():
        data = value.encode("utf-8") + b"\0"
        body += ids[key] + struct.pack("<I", len(data)) + data + (b"\0" if len(data) % 2 else b"")
    with open(path, "r+b") as fh:
        fh.seek(0, 2)
        fh.write(b"LIST" + struct.pack("<I", len(body)) + body)
        size = fh.tell() - 8
        fh.seek(4)
        fh.write(struct.pack("<I", size))


SMOKE_LRC = "[00:00.50] Smoke test line one\n[00:01.50] Smoke test line two"


def fake_lrclib():
    """A one-song LRCLIB on 127.0.0.1."""
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *a):
            pass

        def do_GET(self):
            found = self.path.startswith("/api/get?") and "Smoke+Song" in self.path
            body = json.dumps({"id": 1, "trackName": "Smoke Song", "artistName": "Smoke Band", "albumName": "",
                               "duration": 3.0, "instrumental": False, "plainLyrics": "Smoke test line one",
                               "syncedLyrics": SMOKE_LRC} if found else {"message": "not found"}).encode()
            self.send_response(200 if found else 404)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def lyrics_smoke(cli: str, env: dict, tmp: str, noise) -> int:
    folder = os.path.join(tmp, "lyrics")
    os.makedirs(folder)
    song = os.path.join(folder, "Smoke Song.wav")
    write_wav(song, (noise[: 44100 * 3] * 32767).round().astype("int64"), 44100, 2)
    add_info_tags(song, title="Smoke Song", artist="Smoke Band")
    server = fake_lrclib()
    env = dict(env, REALHIRES_LYRICS_INTERVAL="0", NO_PROXY="127.0.0.1,localhost", no_proxy="127.0.0.1,localhost")
    run = lambda *a: subprocess.run([cli, "--no-settings", "--no-color", *a], capture_output=True, text=True,  # noqa: E731
                                    encoding="utf-8", errors="replace", env=env, timeout=300)
    proc = run(folder, "--lyrics-only", "--set", f"lyrics_server=http://127.0.0.1:{server.server_address[1]}")
    server.shutdown()
    print(proc.stdout)
    print(proc.stderr, file=sys.stderr)
    lrc = os.path.join(folder, "Smoke Song.lrc")
    got = open(lrc, encoding="utf-8").read() if os.path.isfile(lrc) else None
    if proc.returncode != 0 or got != SMOKE_LRC + "\n":
        print(f"Lyrics were not saved as expected (exit {proc.returncode}): {got!r}", file=sys.stderr)
        return 1
    proc = run("--check-lyrics-server")
    print(proc.stdout)
    print(proc.stderr, file=sys.stderr)
    text = (proc.stdout + proc.stderr).lower()
    if proc.returncode != 0 and ("securely" in text or "certificate" in text):
        print("The frozen CLI cannot verify lrclib.net's certificate", file=sys.stderr)
        return 1
    if proc.returncode != 0:
        print("warning: lrclib.net could not be reached from this runner; the certificate check was skipped",
              file=sys.stderr)
    return 0


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
    if lyrics_smoke(cli, env, tmp, noise):
        return 1
    print("Smoke test passed (saved results reused on the second run, lyrics saved):", got)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
