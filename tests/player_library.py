"""A small music library for the player's GUI test and the README screenshots:
invented albums, songs and lyrics, covers drawn by FFmpeg, audio generated
(a quiet tone, or silence for long songs, so the files stay small)."""

import subprocess
from pathlib import Path

ALBUMS = [
    # artist, album, genre, year, rate, bits, colours, songs (title, seconds)
    ("Aurora Fields", "Northern Lights", "Ambient", "2021", 96000, 24, ("0x14213d", "0x4cc9f0"),
     [("First Light", 214), ("Glacier Song", 251), ("Midnight Sun", 198), ("Polar Drift", 305)]),
    ("The Paper Boats", "City Rain", "Indie Pop", "2019", 44100, 16, ("0xff006e", "0xffbe0b"),
     [("Umbrella Weather", 187), ("Taxi Lights", 203), ("Last Train Home", 242)]),
    ("June Harbor", "Small Hours", "Jazz", "2018", 48000, 24, ("0x2a9d8f", "0xe9c46a"),
     [("Blue Kettle", 276), ("Porch Lamp", 231)]),
    ("Velvet Static", "Signal & Noise", "Electronic", "2023", 44100, 16, ("0x7209b7", "0xf72585"),
     [("Carrier Wave", 222), ("Low Pass", 190)]),
]

# invented lyrics for "Glacier Song" (synced) and "Umbrella Weather" (plain)
GLACIER_LRC = """[ar: Aurora Fields]
[ti: Glacier Song]
[00:00.50] Blue ice under a silver sky
[00:04.00] Every footstep answers back
[00:07.50] The wind is writing on the snow
[00:11.00] A letter no one will read
[00:14.50] Hold the lantern a little higher
[00:18.00] Count the stars until they fade
[00:21.50] The glacier sings a slow, low song
[00:25.00] And morning finds us here
[00:28.50] Blue ice under a golden sky
[00:32.00] Every footstep carries home"""

UMBRELLA_TXT = """Puddles on the avenue
Neon in the rain
We share a coat, we share a song
And wait for the late train"""


def make_library(root: Path, seconds=None, mp3: bool = True) -> dict:
    """The library under root/Music; `seconds` replaces every song's length (short
    test songs hold a quiet tone, the real lengths hold silence).  Returns
    {"music": folder, "songs": {title: path}, "albums": [folder, ...]}."""
    music = root / "Music"
    out = {"music": str(music), "songs": {}, "albums": []}
    for artist, album, genre, year, rate, bits, (c0, c1), songs in ALBUMS:
        folder = music / artist / album
        folder.mkdir(parents=True, exist_ok=True)
        out["albums"].append(str(folder))
        subprocess.run(["ffmpeg", "-v", "error", "-y", "-f", "lavfi", "-i",
                        f"gradients=s=600x600:c0={c0}:c1={c1}:x0=0:y0=0:x1=600:y1=600:nb_colors=2:d=1",
                        "-frames:v", "1", str(folder / "cover.png")], check=True)
        lossy = mp3 and genre == "Electronic"
        for n, (title, length) in enumerate(songs, start=1):
            ext = ".mp3" if lossy else ".flac"
            path = folder / f"{n:02d} {title}{ext}"
            secs = seconds if seconds is not None else length
            src = (f"sine=frequency={220 * n}:sample_rate={rate}:duration={secs}" if seconds is not None
                   else f"anullsrc=r={rate}:cl=stereo")
            cmd = ["ffmpeg", "-v", "error", "-y", "-f", "lavfi", "-i", src]
            if seconds is None:
                cmd += ["-t", str(secs)]
            for k, v in (("title", title), ("artist", artist), ("album", album), ("album_artist", artist),
                         ("genre", genre), ("date", year), ("track", f"{n}/{len(songs)}")):
                cmd += ["-metadata", f"{k}={v}"]
            if lossy:
                cmd += ["-ac", "2", "-c:a", "libmp3lame", "-b:a", "256k"]
            else:
                cmd += ["-ac", "2", "-c:a", "flac", "-sample_fmt", "s32" if bits > 16 else "s16"]
                if bits > 16:
                    cmd += ["-bits_per_raw_sample", "24"]
            subprocess.run(cmd + [str(path)], check=True)
            out["songs"][title] = str(path)
    Path(out["songs"]["Glacier Song"]).with_suffix(".lrc").write_text(GLACIER_LRC + "\n", encoding="utf-8")
    Path(out["songs"]["Umbrella Weather"]).with_suffix(".txt").write_text(UMBRELLA_TXT + "\n", encoding="utf-8")
    return out
