"""Writing lyrics into an audio file (2.0) keeps its saved scan result: the
audio is unchanged, so the file must not be analysed again."""

import shutil
import subprocess

import pytest

from realhires import embed, lyrics, store
from realhires.settings import Settings
from realhires.verdict import analyze_file

needs_ffmpeg = pytest.mark.skipif(shutil.which("ffmpeg") is None, reason="ffmpeg not installed")


@needs_ffmpeg
def test_embedding_lyrics_keeps_the_saved_result(tmp_path):
    song = tmp_path / "song.flac"
    subprocess.run(["ffmpeg", "-v", "error", "-f", "lavfi", "-i", "sine=frequency=440:duration=2",
                    "-metadata", "title=Paper Lanterns", "-metadata", "artist=The Invented",
                    str(song)], check=True)
    cfg = Settings()
    cfg.lyrics_save_as = "Inside the audio file (MP3, FLAC)"
    skey = store.settings_key(cfg)
    st = store.open_store(cfg)
    st.put(analyze_file(str(song), settings=cfg), skey)
    assert st.lookup(str(song), skey) is not None
    synced = "[00:00.50] Lanterns over water\n[00:01.20] Paper in the rain\n"
    out = lyrics.save_lyrics(str(song), lyrics.Lyrics(synced=synced, plain="", instrumental=False, how="exact"),
                             cfg)
    assert out.status == lyrics.SYNCED and str(song) in out.written, out.message
    assert embed.embedded_lyrics(str(song))[0].strip() == synced.strip()
    again = st.lookup(str(song), skey)
    assert again is not None, "the saved result was dropped although only the tags changed"
    st.close()
