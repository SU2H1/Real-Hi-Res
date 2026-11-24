"""Synced lyrics from LRCLIB (realhires/lyrics.py), against a fake LRCLIB server on 127.0.0.1."""

import json
import os
import random
import re
import shutil
import ssl
import subprocess
import sys
import threading
import urllib.error
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

import pytest

from realhires import __version__, cli, lyrics
from realhires.lyrics import (INSTRUMENTAL_LRC, MIN_GAP_S, MIN_INTERVAL_S, LrclibClient, LyricsError, Track,
                              best_match, find_lyrics, parse_record)
from realhires.settings import Settings

needs_ffmpeg = pytest.mark.skipif(shutil.which("ffmpeg") is None, reason="ffmpeg not installed")

SYNCED_A = "[00:00.50] First line of Alpha\n[00:02.00] Second line of Alpha"
SYNCED_B = "[00:01.00] Beta goes on"


def rec(title, artist="The Band", album="First", duration=4.0, synced=None, plain=None, instrumental=False, rid=1):
    return {"id": rid, "trackName": title, "artistName": artist, "albumName": album, "duration": duration,
            "instrumental": instrumental, "syncedLyrics": synced,
            "plainLyrics": plain if plain is not None else (lyrics.strip_timestamps(synced) if synced else None)}


class FakeLrclib:
    """/api/get (exact title, artist, album, ±2 s) and /api/search (title keyword), like LRCLIB."""

    def __init__(self):
        self.songs = []
        self.requests = []      # (path, params, user agent)
        self.answers = []       # (status, headers) to send before the normal answers
        fake = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def _send(self, status, body, headers=None):
                data = json.dumps(body).encode()
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
                for k, v in (headers or {}).items():
                    self.send_header(k, v)
                self.end_headers()
                self.wfile.write(data)

            def do_GET(self):
                u = urlsplit(self.path)
                q = {k: v[0] for k, v in parse_qs(u.query).items()}
                fake.requests.append((u.path, q, self.headers.get("User-Agent", "")))
                if fake.answers:
                    status, headers = fake.answers.pop(0)
                    return self._send(status, {"name": "Error", "message": "fake", "statusCode": status}, headers)
                if u.path == "/api/get":
                    for r in fake.songs:
                        if (r["trackName"].casefold() == q.get("track_name", "").casefold()
                                and r["artistName"].casefold() == q.get("artist_name", "").casefold()
                                and ("album_name" not in q or r["albumName"].casefold() == q["album_name"].casefold())
                                and ("duration" not in q or abs(r["duration"] - float(q["duration"])) <= 2)):
                            return self._send(200, r)
                    return self._send(404, {"message": "Failed to find specified track", "name": "TrackNotFound",
                                            "statusCode": 404})
                if u.path == "/api/search":
                    word = q.get("track_name", "").casefold()
                    return self._send(200, [r for r in fake.songs if word and word in r["trackName"].casefold()])
                return self._send(404, {"message": "no such endpoint"})

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}"

    def looked_up(self):
        return [q.get("track_name") for p, q, _ in self.requests if p == "/api/get"]


@pytest.fixture
def lrclib(monkeypatch):
    monkeypatch.setenv(lyrics.INTERVAL_ENV, "0")  # no pauses for the local server (never for lrclib.net)
    for var in ("NO_PROXY", "no_proxy"):
        monkeypatch.setenv(var, "127.0.0.1,localhost")
    fake = FakeLrclib()
    yield fake
    fake.server.shutdown()


def tagged(path, seconds=4, **tags):
    cmd = ["ffmpeg", "-v", "error", "-y", "-f", "lavfi", "-i", f"sine=frequency=440:duration={seconds}"]
    for k, v in tags.items():
        cmd += ["-metadata", f"{k}={v}"]
    subprocess.run(cmd + ["-c:a", "flac", str(path)], check=True)
    return str(path)


def make_album(folder: Path, fake: FakeLrclib):
    folder.mkdir()
    band = dict(artist="The Band", album="First")
    tagged(folder / "01 Alpha.flac", title="Alpha", **band)
    tagged(folder / "02 Beta.flac", title="Beta", artist="The Band", album="First (Deluxe Edition)")
    tagged(folder / "03 Gamma.flac", title="Gamma", **band)
    tagged(folder / "04 Delta.flac", title="Delta", **band)
    tagged(folder / "05 Epsilon.flac", title="Epsilon", **band)
    tagged(folder / "06 untagged.flac")
    tagged(folder / "07 Zeta.flac", title="Zeta", **band)
    fake.songs += [rec("Alpha", synced=SYNCED_A, rid=1),
                   rec("Beta", synced=SYNCED_B, rid=2),                     # found by searching
                   rec("Gamma", plain="Gamma has no timing", rid=3),        # plain only -> .txt
                   rec("Delta", instrumental=True, rid=4),                  # instrumental mark
                   rec("Zeta", synced="[00:01.00] wrong song", duration=9.0, rid=5)]  # length differs


def run_cli(capsys, *args):
    rc = cli.main(["--no-settings", "--no-color", *args])
    out = capsys.readouterr()
    return rc, out.out + out.err


# ------------------------------------------------------------------ client ---
def test_lrclib_client_is_polite(monkeypatch):
    """Names itself, honours Retry-After with at most 30 % jitter, one request at a time with pauses."""
    now, calls, sleeps = [100.0], [], []
    answers = [(429, {"retry-after": "2"}, b'{"name": "RateLimitExceeded"}'),
               (200, {}, json.dumps(rec("Song", "Band", "Album", 200.0, synced="[00:01.00] la")).encode()),
               (404, {}, b"{}")]

    def transport(url, headers, timeout):
        calls.append((now[0], url, headers))
        now[0] += 0.1  # each answer takes 0.1 s
        return answers.pop(0)

    def sleep(s):
        sleeps.append(s)
        now[0] += s

    c = LrclibClient("https://lrclib.net", transport=transport, clock=lambda: now[0], sleep=sleep,
                     rng=random.Random(1))
    lyr = find_lyrics(c, Track("", "Song", "Band", "Album", 200.4))
    assert lyr is not None and lyr.synced and lyr.how == "exact"  # synced: no search needed
    assert calls[0][2]["User-Agent"] == f"Real-Hi-Res v{__version__} (https://github.com/SU2H1/Real-Hi-Res)"
    q = {k: v[0] for k, v in parse_qs(urlsplit(calls[0][1]).query).items()}
    assert q == {"track_name": "Song", "artist_name": "Band", "album_name": "Album", "duration": "200"}
    waited = calls[1][0] - (calls[0][0] + 0.1)
    assert 2.0 <= waited <= 2.6 + 1e-9, waited
    assert c.get(Track("", "Other", "Band")) is None  # 404 = not found, not an error
    assert calls[2][0] - calls[1][0] >= MIN_INTERVAL_S - 1e-9
    assert calls[2][0] - (calls[1][0] + 0.1) >= MIN_GAP_S - 1e-9
    # lrclib.net itself never gets requests faster than MIN_INTERVAL_S, whatever the environment says
    monkeypatch.setenv(lyrics.INTERVAL_ENV, "0")
    assert lyrics.default_interval("https://lrclib.net") == MIN_INTERVAL_S
    assert lyrics.default_interval("http://127.0.0.1:8080") == 0
    assert LrclibClient("lrclib.net").server == "https://lrclib.net"


def test_lrclib_client_gives_up_and_keeps_waiting(monkeypatch):
    now, calls = [0.0], []

    def transport(url, headers, timeout):
        calls.append(now[0])
        return 503, {"retry-after": "5"}, b""

    c = LrclibClient("https://lrclib.net", transport=transport, clock=lambda: now[0],
                     sleep=lambda s: now.__setitem__(0, now[0] + s), rng=random.Random(2))
    with pytest.raises(LyricsError, match="after 4 attempts"):
        c.get(Track("", "Song", "Band"))
    assert len(calls) == lyrics.MAX_ATTEMPTS
    assert all(b - a >= 5 for a, b in zip(calls, calls[1:]))
    # the next song still waits for the last Retry-After
    last = now[0]
    calls.clear()
    with pytest.raises(LyricsError):
        c.get(Track("", "Next", "Band"))
    assert calls[0] - last >= 5
    bad = LrclibClient("http://127.0.0.1:1", transport=lambda u, h, t: (400, {}, b'{"message": "duration: bad"}'))
    with pytest.raises(LyricsError, match="refused the request .HTTP 400: duration: bad"):
        bad.get(Track("", "Song", "Band"))

    def untrusted(url, headers, timeout):  # what urllib raises when no CA certificate matches
        raise urllib.error.URLError(ssl.SSLCertVerificationError(1, "certificate verify failed"))

    c = LrclibClient("https://lrclib.net", transport=untrusted, sleep=lambda s: None)
    with pytest.raises(LyricsError, match="Could not connect to lrclib.net securely"):
        c.get(Track("", "Song", "Band"))
    assert c.requests == 1  # not retried: it cannot get better by waiting


def test_records_and_search_matching():
    assert parse_record(rec("A", synced=SYNCED_A)).synced == SYNCED_A
    assert parse_record(rec("A", synced=SYNCED_A)).plain == "First line of Alpha\nSecond line of Alpha"
    assert parse_record(rec("A", plain="words")).plain == "words"
    inst = parse_record(rec("A", instrumental=True))
    assert inst.instrumental and not inst.synced and not inst.plain
    assert parse_record(rec("A")) is None and parse_record("nonsense") is None
    track = Track("", "Song", "Queen;David Bowie", "Album", 200.0)
    found = [rec("Song", "Queen", duration=205.0, synced="[00:01.00] too long", rid=1),
             rec("Song", "Someone Else", duration=200.0, synced="[00:01.00] other artist", rid=2),
             rec("Song", "Queen", duration=201.5, plain="plain words", rid=3),
             rec("Song (Remastered 2011)", "Queen & David Bowie", duration=199.0, synced="[00:01.00] yes", rid=4),
             rec("Another Song", "Queen", duration=200.0, synced="[00:01.00] other title", rid=5)]
    assert best_match(found, track).lrclib_id == 4      # synced, same length, same title and artist
    assert best_match(found[:3], track).lrclib_id == 3  # else plain lyrics of the same song
    assert best_match(found[:2], track) is None
    # without a length only the same album is trusted
    assert best_match(found, Track("", "Song", "Queen", "Other album")) is None
    assert lyrics.normalize_name("夜に駆ける") == "夜に駆ける"
    assert Track("", "Song", "A;B").params()["artist_name"] == "A, B"


# --------------------------------------------------------------------- CLI ---
@needs_ffmpeg
def test_cli_downloads_lyrics_like_lrcget(lrclib, tmp_path, capsys):
    folder = tmp_path / "album"
    make_album(folder, lrclib)
    server = f"lyrics_server={lrclib.url}"
    rc, out = run_cli(capsys, str(folder), "--lyrics-only", "--set", server)
    assert rc == 0, out
    assert (folder / "01 Alpha.lrc").read_text(encoding="utf-8") == SYNCED_A + "\n"
    assert (folder / "02 Beta.lrc").read_text(encoding="utf-8") == SYNCED_B + "\n"
    assert (folder / "03 Gamma.txt").read_text(encoding="utf-8") == "Gamma has no timing\n"
    assert not (folder / "03 Gamma.lrc").exists()
    assert (folder / "04 Delta.lrc").read_text(encoding="utf-8") == INSTRUMENTAL_LRC + "\n"
    for name in ("05 Epsilon", "06 untagged", "07 Zeta"):  # Zeta's only record is 5 s longer
        assert not (folder / f"{name}.lrc").exists() and not (folder / f"{name}.txt").exists()
    assert "(found by searching)" in out
    assert ("Lyrics: 2 with synced lyrics, 1 with plain lyrics (.txt), 1 instrumental, 2 not found on LRCLIB, "
            "1 without title/artist tags.") in out, out
    assert all(ua.startswith("Real-Hi-Res v") for _, _, ua in lrclib.requests)
    first = next(q for p, q, _ in lrclib.requests if p == "/api/get" and q["track_name"] == "Alpha")
    assert first == {"track_name": "Alpha", "artist_name": "The Band", "album_name": "First", "duration": "4"}
    assert not [f for f in os.listdir(folder) if f.startswith(".realhires-")]  # no temp files left

    # again: songs with an .lrc are skipped without a request; Gamma (.txt only) is asked for synced lyrics
    lrclib.requests.clear()
    rc, out = run_cli(capsys, str(folder), "--lyrics-only", "--set", server)
    assert rc == 0, out
    assert sorted(set(lrclib.looked_up())) == ["Epsilon", "Gamma", "Zeta"]
    assert "Lyrics: 2 not found on LRCLIB, 4 skipped (already had lyrics), 1 without title/artist tags." in out
    lrclib.requests.clear()
    rc, out = run_cli(capsys, str(folder), "--lyrics-only", "--lyrics-existing", "skip-any", "--set", server)
    assert sorted(set(lrclib.looked_up())) == ["Epsilon", "Zeta"]

    # replace: downloaded again; a dry run saves nothing
    lrclib.songs[0]["syncedLyrics"] = "[00:00.10] Alpha, corrected"
    rc, out = run_cli(capsys, str(folder / "01 Alpha.flac"), "--lyrics-only", "--lyrics-existing", "replace",
                      "--dry-run", "--set", server)
    assert "Would save" in out and (folder / "01 Alpha.lrc").read_text(encoding="utf-8") == SYNCED_A + "\n"
    rc, out = run_cli(capsys, str(folder / "01 Alpha.flac"), "--lyrics-only", "--lyrics-existing", "replace",
                      "--set", server)
    assert (folder / "01 Alpha.lrc").read_text(encoding="utf-8") == "[00:00.10] Alpha, corrected\n"

    # after a scan, with the tags of the scan results
    song = tagged(tmp_path / "Eta.flac", title="Eta", artist="The Band", album="First")
    lrclib.songs.append(rec("Eta", synced="[00:00.20] Eta sings", rid=6))
    rc, out = run_cli(capsys, song, "--lyrics", "--set", server)
    assert rc == 0 and "NOT HI-RES" in out and "Lyrics: 1 with synced lyrics." in out, out
    assert (tmp_path / "Eta.lrc").read_text(encoding="utf-8") == "[00:00.20] Eta sings\n"


@needs_ffmpeg
def test_cli_reports_lyrics_failures(lrclib, tmp_path, capsys):
    song = tagged(tmp_path / "Song.flac", title="Song", artist="Band")
    lrclib.answers = [(400, {})]
    rc, out = run_cli(capsys, song, "--lyrics-only", "--set", f"lyrics_server={lrclib.url}")
    assert rc == 1 and "Failed" in out and "refused the request (HTTP 400: fake)" in out, out
    assert "Lyrics: 1 failed." in out
    rc, out = run_cli(capsys, "--check-lyrics-server", "--set", f"lyrics_server={lrclib.url}")
    assert rc == 0 and "can be reached" in out, out
    lrclib.answers = [(400, {})]
    rc, out = run_cli(capsys, "--check-lyrics-server", "--set", f"lyrics_server={lrclib.url}")
    assert rc == 1 and "error:" in out


def test_skip_rules(tmp_path):
    song = tmp_path / "Song.flac"
    song.write_bytes(b"")
    cfg = Settings()
    assert lyrics.skip_reason(str(song), cfg) == ""
    (tmp_path / "Song.txt").write_text("words", encoding="utf-8")
    assert lyrics.skip_reason(str(song), cfg) == ""  # a .txt does not stop the search for synced lyrics
    cfg.lyrics_existing = lyrics.EXISTING_CLI["skip-any"]
    assert lyrics.skip_reason(str(song), cfg) == "Already has lyrics (Song.txt)."
    (tmp_path / "Song.lrc").write_text(INSTRUMENTAL_LRC, encoding="utf-8")
    cfg.lyrics_existing = lyrics.EXISTING_CLI["skip-synced"]
    assert lyrics.skip_reason(str(song), cfg) == "Already has an instrumental mark (Song.lrc)."
    cfg.lyrics_existing = lyrics.EXISTING_CLI["replace"]
    assert lyrics.skip_reason(str(song), cfg) == ""
    assert lyrics.read_lyrics_file(str(song))[0] == lyrics.INSTRUMENTAL


def test_lrclib_and_lrcget_are_credited():
    """The user asked for the lyrics code to be credited: LRCLIB and LRCGET (both MIT, by tranxuanthang)
    are named in both apps (lyrics window, About box), the packaged READMEs, the README and the
    release notes, and the Rust version shows the same texts."""
    root = Path(__file__).resolve().parent.parent
    for name in ("README.md", "packaging/README-RELEASE.txt", "packaging/README-RUST.txt"):
        text = (root / name).read_text(encoding="utf-8")
        for needed in ("lrclib.net", "github.com/tranxuanthang/lrcget", "Copyright (c) 2023 tranxuanthang",
                       "Copyright (c) 2024 tranxuanthang", "MIT License"):
            assert needed in text, f"{name} lacks {needed!r}"
    notes = (root / "packaging" / "RELEASE_NOTES.md").read_text(encoding="utf-8")
    assert "LRCGET by tranxuanthang" in notes and "MIT-licensed" in notes
    assert "LRCLIB" in lyrics.CREDIT and "LRCGET by tranxuanthang" in lyrics.CREDIT
    assert "LRCGET by tranxuanthang (MIT License)" in lyrics.ABOUT_CREDIT
    rust = (root / "rust" / "crates" / "core" / "src" / "lyrics.rs").read_text(encoding="utf-8")

    def rust_const(name):
        m = re.search(r'pub const %s: &str = "((?:[^"\\]|\\.)*)";' % name, rust, re.S)
        assert m, name
        return re.sub(r"\\\n\s*", "", m.group(1))  # a backslash at a line end joins the lines

    assert rust_const("CREDIT") == lyrics.CREDIT
    assert rust_const("ABOUT_CREDIT") == lyrics.ABOUT_CREDIT
    assert "CREDIT.into()" in (root / "rust" / "crates" / "gui" / "src" / "lyrics.rs").read_text(encoding="utf-8")
    assert "ABOUT_CREDIT" in (root / "rust" / "crates" / "gui" / "src" / "app.rs").read_text(encoding="utf-8")


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
def test_gui_lyrics_window(lrclib, tmp_path):
    folder = tmp_path / "album"
    make_album(folder, lrclib)
    script = tmp_path / "drive.py"
    script.write_text(f"""
import faulthandler, os, sys
sys.path.insert(0, {str(Path(__file__).resolve().parents[1])!r})
faulthandler.dump_traceback_later(100, exit=True)
def run():
    import tkinter as tk
    from realhires import gui
    root = tk.Tk()
    def fail(exc, val, tb):  # an assertion in a Tk callback must end the test, not hang it
        import traceback
        traceback.print_exception(exc, val, tb)
        os._exit(1)
    root.report_callback_exception = fail
    folder = {str(folder)!r}
    app = gui.App(root)
    app.cfg.lyrics_server = {lrclib.url!r}
    app._add_source(folder)
    alpha = os.path.join(folder, "01 Alpha.flac")
    def listed():
        if app.listing or app.lookup_busy:
            return root.after(100, listed)
        app.tree.selection_set(alpha); root.update()
        assert "No lyrics next to this song yet" in app.lyrics_text.get("1.0", "end")
        assert str(app.lyrics_btn.cget("state")) == "normal"
        app.lyrics_all(); root.update()
        d = app.lyrics_dialog
        assert len(d.todo) == 7 and d.head.cget("text") == "Download synced lyrics for 7 songs", d.head.cget("text")
        d._start()
        assert d.running
        app.lyrics_all()  # a second window must not start while this one downloads
        assert app.lyrics_dialog is d and "already" in app.status_var.get()
        root.after(100, done)
    def done():
        d = app.lyrics_dialog
        if d.running:
            return root.after(100, done)
        assert len(d.outcomes) == 7 and len(d.saved_paths) == 4, [o.message for o in d.outcomes]
        assert d.tv.set(alpha, "saved") == "01 Alpha.lrc" and d.tv.set(alpha, "status") == "Synced lyrics saved"
        assert "ok" in d.tv.item(alpha, "tags")
        d.tv.selection_set(alpha); root.update()
        assert "Saved synced lyrics as 01 Alpha.lrc." in d.detail.cget("text"), d.detail.cget("text")
        assert "2 with synced lyrics" in d.status.cget("text"), d.status.cget("text")
        text = app.lyrics_text.get("1.0", "end")  # the selected song's lyrics appeared at once
        assert "Synced lyrics · 01 Alpha.lrc" in text and "First line of Alpha" in text, text
        assert "[00:00.50]" in text
        # Ctrl/Cmd+L on a song that has lyrics now: nothing to look up
        app.lyrics_selected(); root.update()
        assert app.lyrics_dialog is not d and app.lyrics_dialog.head.cget("text") == "Nothing to look up"
        print("OK")
        app.quit_app()
    root.after(300, listed)
    root.mainloop()
if __name__ == "__main__":
    run()
""", encoding="utf-8")  # the script holds non-ASCII text; Windows would write it as cp1252
    env = dict(os.environ, REALHIRES_SETTINGS=str(tmp_path / "gui-settings.json"))
    proc = subprocess.run([sys.executable, str(script)], capture_output=True, text=True, encoding="utf-8",
                          errors="replace", timeout=150, env=env)
    log = proc.stdout + proc.stderr
    assert proc.returncode == 0, log
    assert proc.stdout.splitlines()[-1] == "OK", log
    assert (folder / "01 Alpha.lrc").read_text(encoding="utf-8") == SYNCED_A + "\n"
