"""Album covers kept on disk (realhires/covers.py: cache_key/cache_name,
cached_cover_png, trim_cache)."""

import hashlib
import os
import shutil
from pathlib import Path

import pytest

from realhires import covers

needs_ffmpeg = pytest.mark.skipif(shutil.which("ffmpeg") is None, reason="ffmpeg not installed")
ICON = str(Path(__file__).resolve().parents[1] / "realhires" / "assets" / "icon.png")


def test_cache_key_and_name_are_spelled_out():
    paths = ["/no/such/Music/A/01.flac", "/no/such/Music/A/02.flac", "/no/such/Music/B/03.flac",
             "/no/such/Music/A/04.flac", "/no/such/Music/C/05.flac"]
    key = covers.cache_key(paths, 176)
    assert key == ("v1\t176\n"
                   "/no/such/Music/A/01.flac\t-1\t-1\n/no/such/Music/A/02.flac\t-1\t-1\n"
                   "/no/such/Music/B/03.flac\t-1\t-1\n/no/such/Music/A/04.flac\t-1\t-1\n"
                   "/no/such/Music/A\t-1\n/no/such/Music/B\t-1")  # the fifth song and its folder are not used
    name = covers.cache_name(paths, 176)
    assert name == hashlib.sha1(key.encode("utf-8")).hexdigest() + ".png"
    assert covers.cache_name(paths[:4], 176) == name
    assert covers.cache_name(paths, 220) != name
    assert covers.cache_name(["/no/such/日本語/曲.flac"], 44).endswith(".png")


def test_cache_key_uses_sizes_and_times(tmp_path):
    album = tmp_path / "Album"
    album.mkdir()
    song = album / "01.flac"
    song.write_bytes(b"x" * 1234)
    os.utime(song, ns=(1_700_000_000_000_000_000, 1_700_000_001_000_000_000))
    os.utime(album, ns=(1_700_000_000_000_000_000, 1_700_000_002_000_000_000))
    assert covers.cache_key([str(song)], 150) == (f"v1\t150\n{song}\t1234\t1700000001000000000\n"
                                                   f"{album}\t1700000002000000000")


def test_cache_dir_follows_the_environment(monkeypatch, tmp_path):
    assert covers.cache_dir() == str(tmp_path / "covers")  # conftest keeps tests away from the user's folder
    monkeypatch.delenv("REALHIRES_COVERS")
    from realhires.store import data_dir

    assert covers.cache_dir() == os.path.join(data_dir(), "covers")


def _album(root: Path, name: str, songs: int = 2, image: bool = True):
    """WAV songs (no embedded picture) and, optionally, a folder.png."""
    d = root / name
    d.mkdir(parents=True)
    paths = []
    for n in range(1, songs + 1):
        p = d / f"{n:02d}.wav"
        p.write_bytes(_WAV)
        paths.append(str(p))
    if image:
        shutil.copy(ICON, d / "folder.png")
    return paths


def _wav() -> bytes:
    import struct

    data = b"\x00\x00" * 800
    return (b"RIFF" + struct.pack("<I", 36 + len(data)) + b"WAVEfmt " +
            struct.pack("<IHHIIHH", 16, 1, 1, 8000, 16000, 2, 16) + b"data" + struct.pack("<I", len(data)) + data)


_WAV = _wav()


@pytest.fixture
def ffmpeg_runs(monkeypatch):
    """Counts the FFmpeg runs of covers._thumbnail."""
    runs = []
    real = covers._thumbnail

    def counting(source, size, embedded):
        runs.append(source)
        return real(source, size, embedded)

    monkeypatch.setattr(covers, "_thumbnail", counting)
    return runs


@needs_ffmpeg
def test_a_cover_is_made_once_then_read_from_disk(tmp_path, ffmpeg_runs):
    paths = _album(tmp_path / "Music", "Album")
    png = covers.cached_cover_png(paths, 64)
    assert png and png.startswith(b"\x89PNG") and ffmpeg_runs
    file = Path(covers.cache_file(paths, 64))
    assert file.read_bytes() == png and file.parent == tmp_path / "covers"
    assert not [n for n in os.listdir(file.parent) if n.endswith(".tmp")]
    os.utime(file, (1_000_000, 1_000_000))
    made = len(ffmpeg_runs)
    assert covers.cached_cover_png(paths, 64) == png
    assert len(ffmpeg_runs) == made  # read from disk, no FFmpeg
    assert file.stat().st_mtime > 1_000_000  # reading touched it, for trim_cache()
    assert covers.cached_cover_png(paths, 32) != png and len(ffmpeg_runs) > made  # one file per size


@needs_ffmpeg
def test_an_album_without_cover_is_remembered(tmp_path, ffmpeg_runs):
    paths = _album(tmp_path / "Music", "Plain", image=False)
    assert covers.cached_cover_png(paths, 64) is None
    file = Path(covers.cache_file(paths, 64))
    assert file.is_file() and file.stat().st_size == 0 and covers.is_cached(paths, 64)
    made = len(ffmpeg_runs)
    assert covers.cached_cover_png(paths, 64) is None and len(ffmpeg_runs) == made


def test_no_mark_when_ffmpeg_could_not_run(tmp_path, monkeypatch):
    monkeypatch.setattr(covers, "tool", lambda name: str(tmp_path / "no-ffmpeg"))
    paths = _album(tmp_path / "Music", "Album")
    assert covers.cached_cover_png(paths, 64) is None
    assert not covers.is_cached(paths, 64)  # tried again next time, when FFmpeg is there


@needs_ffmpeg
def test_a_changed_song_or_folder_gives_a_new_entry(tmp_path, ffmpeg_runs):
    paths = _album(tmp_path / "Music", "Album", songs=4)
    first = covers.cache_name(paths, 64)
    covers.cached_cover_png(paths, 64)
    made = len(ffmpeg_runs)
    os.utime(paths[1], ns=(1_600_000_000_000_000_000, 1_600_000_000_000_000_000))  # a re-tagged song
    second = covers.cache_name(paths, 64)
    assert second != first
    covers.cached_cover_png(paths, 64)
    assert len(ffmpeg_runs) > made
    folder = os.path.dirname(paths[0])
    os.utime(folder, ns=(1_600_000_000_000_000_000, 1_600_000_005_000_000_000))  # e.g. a new cover.jpg
    assert covers.cache_name(paths, 64) not in (first, second)
    assert covers.cache_name(paths + ["/elsewhere/05.flac"], 64) == covers.cache_name(paths, 64)


@needs_ffmpeg
def test_a_damaged_file_is_made_again(tmp_path, ffmpeg_runs):
    paths = _album(tmp_path / "Music", "Album")
    png = covers.cached_cover_png(paths, 64)
    file = Path(covers.cache_file(paths, 64))
    file.write_bytes(png[: len(png) // 2])  # cut short
    made = len(ffmpeg_runs)
    assert covers.cached_cover_png(paths, 64) == png and len(ffmpeg_runs) > made
    assert file.read_bytes() == png
    file.write_bytes(b"not a png at all")
    assert covers.cached_cover_png(paths, 64) == png


def test_trim_cache_deletes_the_least_recently_used(tmp_path):
    d = tmp_path / "trim"
    d.mkdir()
    for i, (name, size) in enumerate((("a.png", 5000), ("b.png", 100), ("c.png", 0), ("d.png", 9000),
                                      ("e.png", 4096))):
        p = d / name
        p.write_bytes(b"x" * size)
        os.utime(p, (1_000_000 + i, 1_000_000 + i))  # a is the oldest
    old_tmp, young_tmp, other = d / ".cover-1.tmp", d / ".cover-2.tmp", d / "notes.txt"
    for p in (old_tmp, young_tmp, other):
        p.write_bytes(b"x")
    os.utime(old_tmp, (1_000_000, 1_000_000))
    # usage in 4 KB blocks: a 8 KB, b 4 KB, c 4 KB, d 12 KB, e 4 KB = 32 KB
    assert covers.trim_cache(32 * 1024, str(d)) == 0
    assert covers.trim_cache(20 * 1024, str(d)) == 2  # a and b go
    assert sorted(os.listdir(d)) == [".cover-2.tmp", "c.png", "d.png", "e.png", "notes.txt"]
    assert covers.trim_cache(0, str(d)) == 3
    assert covers.trim_cache(0, str(tmp_path / "missing")) == 0
