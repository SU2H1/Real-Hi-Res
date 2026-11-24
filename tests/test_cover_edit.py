"""Changing and removing a song's cover (2.1, `tagedit.set_cover()`/`remove_cover()`)
and where a shown cover comes from (`covers.cover_source()`), on real files of every
format FFmpeg here can write: the audio must come out sample for sample the same,
the other tags and the other pictures must stay, the new picture must be the one
FFmpeg (and so the player) shows, byte for byte, and the saved scan results and the
library must follow.  Formats without a place for a picture are left alone."""

import hashlib
import json
import os
import shutil
import struct
import subprocess

import pytest

from realhires import covers, embed, musiclib, store, tagedit
from realhires.probe import probe
from realhires.settings import Settings
from realhires.verdict import analyze_file

needs_ffmpeg = pytest.mark.skipif(shutil.which("ffmpeg") is None, reason="ffmpeg not installed")


def _encoders() -> str:
    try:
        return subprocess.run(["ffmpeg", "-hide_banner", "-encoders"], capture_output=True, text=True).stdout
    except OSError:
        return ""


def make(path, codec_args, tags=None):
    cmd = ["ffmpeg", "-v", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=440:duration=2", "-ac", "2"]
    for k, v in (tags or {"title": "Old", "artist": "Art", "album": "Alb", "comment": "keep me"}).items():
        cmd += ["-metadata", f"{k}={v}"]
    subprocess.run(cmd + codec_args + [str(path)], check=True)
    return str(path)


def picture(path, source, size="300x200") -> bytes:
    spec = f"{source}{':' if '=' in source else '='}s={size}:d=1"
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-f", "lavfi", "-i", spec, "-frames:v", "1", str(path)], check=True)
    with open(path, "rb") as fh:
        return fh.read()


def noise_png(w, h, seed=7) -> bytes:
    """A PNG of random pixels (it does not compress), written by hand."""
    import random
    import zlib

    rng = random.Random(seed)
    raw = b"".join(b"\x00" + bytes(rng.getrandbits(8) for _ in range(w * 3)) for _ in range(h))

    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))

    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))


def pcm(path) -> str:
    out = subprocess.run(["ffmpeg", "-v", "error", "-i", str(path), "-map", "0:a:0", "-f", "s32le", "-"],
                         capture_output=True, check=True).stdout
    return hashlib.sha1(out).hexdigest()


def video_streams(path):
    out = subprocess.run(["ffprobe", "-v", "error", "-show_entries", "stream=codec_type,codec_name:stream_disposition="
                          "attached_pic:stream_tags=comment", "-of", "json", str(path)],
                         capture_output=True, text=True).stdout
    return [(s["codec_name"], s.get("disposition", {}).get("attached_pic"), s.get("tags", {}).get("comment", ""))
            for s in json.loads(out)["streams"] if s["codec_type"] == "video"]


def clean_decode(path) -> str:
    return subprocess.run(["ffmpeg", "-v", "warning", "-nostdin", "-i", str(path), "-map", "0:a", "-f", "null", "-"],
                          capture_output=True, text=True).stderr.strip()


def formats():
    enc = _encoders()
    out = [("flac", ["-c:a", "flac"]), ("m4a", ["-c:a", "aac"]), ("alac.m4a", ["-c:a", "alac"]),
           ("aiff", ["-c:a", "pcm_s16be", "-write_id3v2", "1"]), ("plain.aiff", ["-c:a", "pcm_s16be"])]
    if "libmp3lame" in enc:
        out += [("mp3", ["-c:a", "libmp3lame", "-id3v2_version", "3"]),
                ("v4.mp3", ["-c:a", "libmp3lame", "-id3v2_version", "4"])]
    if "libvorbis" in enc:
        out.append(("ogg", ["-c:a", "libvorbis"]))
    if "libopus" in enc:
        out.append(("opus", ["-c:a", "libopus"]))
    return out


@needs_ffmpeg
@pytest.mark.parametrize("ext,args", formats(), ids=[f[0] for f in formats()])
def test_a_new_cover_keeps_the_audio_and_the_tags(tmp_path, ext, args):
    song = make(tmp_path / f"song.{ext}", args)
    jpeg = picture(tmp_path / "new.jpg", "testsrc")
    png = picture(tmp_path / "new.png", "color=c=green", "40x30")
    before, tags_before = pcm(song), probe(song).tags
    out = tagedit.set_cover([song], jpeg, "image/jpeg")
    assert len(out) == 1 and out[0].ok and out[0].message == "", out[0].message
    assert pcm(song) == before, "the audio changed"
    assert probe(song).tags == tags_before, "the tags changed"
    src = covers.cover_source([song])
    assert src is not None and src.kind == "embedded" and src.path == song
    assert src.data == jpeg, "the picture is not stored as it was given"
    assert (src.format, src.width, src.height, src.mime) == ("JPEG", 300, 200, "image/jpeg")
    assert src.where == f"Inside song.{ext}" and src.details.startswith("300 × 200 pixels · JPEG · ")
    assert covers.cover_png([song], 64) is not None
    assert len(video_streams(song)) == 1
    assert not ext.endswith("aiff") or video_streams(song)[0][2] == "Cover (front)"

    out = tagedit.set_cover([song], png)  # replaces the cover, does not add a second one
    assert out[0].ok, out[0].message
    assert [s[0] for s in video_streams(song)] == ["png"]
    assert covers.cover_source([song]).data == png
    assert pcm(song) == before
    assert ext.endswith(("m4a", "aiff")) or clean_decode(song) == ""  # those two always warn about something

    out = tagedit.remove_cover([song])
    assert out[0].ok and out[0].message == "", out[0].message
    assert video_streams(song) == [] and covers.cover_source([song]) is None
    assert pcm(song) == before and probe(song).tags == tags_before
    out = tagedit.remove_cover([song])
    assert out[0].ok and out[0].message == f"song.{ext} had no cover inside."
    assert tagedit.cover_summary(out, removing=True) == ("The song had no cover inside.", [])
    assert not [n for n in os.listdir(tmp_path) if n.startswith(".realhires")], "a temporary file was left"


def _flac_picture_block(kind, image):
    mime = b"image/png"
    return (struct.pack(">II", kind, len(mime)) + mime + struct.pack(">I", 4) + b"back"
            + struct.pack(">IIIII", 40, 30, 24, 0, len(image)) + image)


@needs_ffmpeg
def test_other_pictures_stay_and_the_front_cover_comes_first(tmp_path):
    song = make(tmp_path / "a.flac", ["-c:a", "flac"])
    back = picture(tmp_path / "back.png", "color=c=red", "40x30")
    with open(song, "rb") as fh:
        data = fh.read()
    with open(song, "wb") as fh:  # a back cover (type 4) and an old front cover (type 3)
        fh.write(embed.flac_rewrite_blocks(data, lambda b: b + [(6, _flac_picture_block(4, back)),
                                                                 (6, _flac_picture_block(3, back))]))
    before = pcm(song)
    jpeg = picture(tmp_path / "new.jpg", "testsrc")
    assert tagedit.set_cover([song], jpeg)[0].ok
    with open(song, "rb") as fh:
        blocks, _ = embed._flac_blocks(fh.read(), 0)
    pics = [b for k, b in blocks if k == 6]
    assert [tagedit.picture_type(p) for p in pics] == [3, 4], "the old front cover must go, the back cover stay"
    assert pics[0] == tagedit.flac_picture(jpeg) and pics[0][-len(jpeg):] == jpeg
    assert covers.cover_source([song]).data == jpeg and pcm(song) == before
    assert tagedit.remove_cover([song])[0].ok
    src = covers.cover_source([song])
    assert src is not None and src.data == back, "only the front cover is removed"


@needs_ffmpeg
@pytest.mark.skipif("libmp3lame" not in _encoders(), reason="this FFmpeg has no MP3 encoder")
def test_mp3_apic_frames_and_an_id3v22_tag(tmp_path):
    song = make(tmp_path / "a.mp3", ["-c:a", "libmp3lame", "-id3v2_version", "3"])
    back = picture(tmp_path / "back.png", "color=c=red", "40x30")
    with open(song, "rb") as fh:
        data = fh.read()
    other = (b"APIC", b"\x00\x00", b"\x00image/png\x00\x04back\x00" + back)  # a back cover
    itunes = (b"APIC", b"\x00\x00", b"\x00image/png\x00\x00\x00" + back)    # "Other", as iTunes writes a cover
    with open(song, "wb") as fh:
        fh.write(embed.mp3_rewrite(data, lambda major, frames: frames + [other, itunes]))
    before = pcm(song)
    jpeg = picture(tmp_path / "new.jpg", "testsrc")
    out = tagedit.set_cover([song], jpeg)
    assert out[0].ok, out[0].message
    with open(song, "rb") as fh:
        major, frames, _ = embed._id3_frames(fh.read())
    apics = [f for f in frames if f[0] == b"APIC"]
    assert major == 3 and [tagedit.apic_type(major, f[1], f[2]) for f in apics] == [3, 4]
    assert apics[0][2] == tagedit.apic_body(jpeg) and pcm(song) == before
    assert probe(song).tags["title"] == "Old"
    # an ID3v2.2 tag cannot be rewritten here: FFmpeg copies the file with the picture
    with open(tmp_path / "plain.mp3", "wb") as fh:
        fh.write(data[embed._id3_frames(data)[2]:])
    tt2 = b"TT2" + (4).to_bytes(3, "big") + b"\x00Two"
    v22 = tmp_path / "v22.mp3"
    v22.write_bytes(b"ID3\x02\x00\x00" + embed._to_syncsafe(len(tt2) + 16) + tt2 + b"\x00" * 16
                    + (tmp_path / "plain.mp3").read_bytes())
    before = pcm(v22)
    out = tagedit.set_cover([str(v22)], jpeg)
    assert out[0].ok, out[0].message
    assert pcm(v22) == before and covers.cover_source([str(v22)]).data == jpeg


@needs_ffmpeg
@pytest.mark.parametrize("ext,codec", [("ogg", "libvorbis"), ("opus", "libopus")])
def test_a_big_cover_spans_several_ogg_pages(tmp_path, ext, codec):
    if codec not in _encoders():
        pytest.skip(f"this FFmpeg has no {codec} encoder")
    song = make(tmp_path / f"a.{ext}", ["-c:a", codec])
    big = noise_png(200, 200)  # 120 KB: the comment header spans two pages
    assert len(big) > 70000
    before = pcm(song)
    out = tagedit.set_cover([song], big)
    assert out[0].ok, out[0].message
    assert pcm(song) == before and clean_decode(song) == ""
    assert covers.cover_source([song]).data == big
    assert tagedit.remove_cover([song])[0].ok
    assert pcm(song) == before and clean_decode(song) == "" and covers.cover_source([song]) is None
    assert probe(song).tags["title"] == "Old"
    with open(song, "rb") as fh:
        assert b"keep me" in fh.read()


@needs_ffmpeg
def test_saved_results_and_the_library_follow(tmp_path):
    song = make(tmp_path / "a.flac", ["-c:a", "flac"])
    cfg = Settings()
    skey = store.settings_key(cfg)
    st = store.open_store(cfg)
    st.put(analyze_file(song, settings=cfg), skey)
    lib = musiclib.Library()
    lib.refresh([song])
    steps = []
    out = tagedit.set_cover([song], picture(tmp_path / "c.jpg", "testsrc"), cfg=cfg,
                            progress=lambda i, n, p: steps.append((i, n)))
    assert out[0].ok and steps == [(0, 1), (1, 1)]
    again = st.lookup(song, skey)
    assert again is not None, "the saved result was dropped although only the cover changed"
    assert again.tags["album"] == "Alb"
    track = next(t for t in lib.tracks() if t.path == os.path.abspath(song))
    assert track.mtime_ns == os.stat(song).st_mtime_ns, "the library did not read the changed file"
    assert tagedit.remove_cover([song], cfg)[0].ok and st.lookup(song, skey) is not None
    st.close()
    lib.close()


@needs_ffmpeg
def test_formats_without_a_place_for_a_picture_are_left_alone(tmp_path):
    jpeg = picture(tmp_path / "c.jpg", "testsrc")
    wav = make(tmp_path / "a.wav", ["-c:a", "pcm_s16le"])
    ape = tmp_path / "b.ape"
    ape.write_bytes(b"MAC \x00\x00")
    damaged = tmp_path / "c.flac"
    damaged.write_bytes(b"fLaC\x00\x00\x00\x22" + b"\x00" * 10)
    flac = make(tmp_path / "d.flac", ["-c:a", "flac"])
    with open(wav, "rb") as fh:
        wav_bytes = fh.read()
    out = tagedit.set_cover([wav, str(ape), str(damaged), flac], jpeg)
    assert [o.ok for o in out] == [False, False, False, True]
    assert out[0].message == "WAV files have no place for a cover picture, so they were left as they are."
    assert out[1].message == ("The cover of APE files cannot be changed here, because FFmpeg can read them but not "
                              "write them.")
    assert out[2].message.startswith("Could not change the cover of c.flac: The FLAC file ends inside")
    with open(wav, "rb") as fh:
        assert fh.read() == wav_bytes
    assert ape.read_bytes() == b"MAC \x00\x00"
    head, lines = tagedit.cover_summary(out, notes=[tagedit.folder_saved_note(str(tmp_path / "cover.jpg"))])
    assert head == "Changed the cover of 1 of 4 songs."
    assert lines == [out[0].message, out[1].message, out[2].message,
                     "The picture was also saved as cover.jpg in the album's folder."]
    out = tagedit.remove_cover([wav, flac, flac])
    head, lines = tagedit.cover_summary(out, removing=True, notes=[tagedit.folder_kept_note("/x/folder.png")])
    assert head == "Removed the cover from 1 of 3 songs."
    assert lines == ["d.flac had no cover inside.", out[0].message,
                     "folder.png in the album's folder was not deleted; it is shown for the songs that have no "
                     "cover inside."]
    bad = tagedit.set_cover([flac], b"GIF89a....", "image/gif")
    assert not bad[0].ok and bad[0].message == "Only JPEG and PNG pictures can be used as a cover."
    assert tagedit.cover_summary(bad) == ("No song was changed.", [bad[0].message])


def test_a_removed_big_cover_does_not_stay_as_padding():
    blocks = [(0, b"\x00" * 34), (4, b"\x00" * 8), (6, struct.pack(">I", 3) + b"x" * 1_200_000), (1, b"\x00" * 10)]
    data = b"fLaC"
    for i, (k, b) in enumerate(blocks):
        data += bytes([k | (0x80 if i == len(blocks) - 1 else 0)]) + len(b).to_bytes(3, "big") + b
    data += b"AUDIO"
    out, n = tagedit.flac_with_cover(data, None)
    assert n == 1 and len(out) == 4 + 38 + 12 + 4 + embed.NEW_PADDING + 5 and out.endswith(b"AUDIO")


def test_summaries():
    ok, none, bad = tagedit.TagOutcome("a", True), tagedit.TagOutcome("b", True, "b had no cover inside."), \
        tagedit.TagOutcome("c", False, "Could not.")
    assert tagedit.cover_summary([ok, ok]) == ("Changed the cover of 2 songs.", [])
    assert tagedit.cover_summary([ok]) == ("Changed the cover of 1 song.", [])
    assert tagedit.cover_summary([], removing=True) == ("No cover was removed.", [])
    assert tagedit.cover_summary([none, none], removing=True) == ("None of the songs had a cover inside.", [])
    assert tagedit.cover_summary([ok, none, none, bad], removing=True) == (
        "Removed the cover from 1 of 4 songs.", ["2 of the songs had no cover inside.", "Could not."])
    assert tagedit.cover_summary([none, bad], removing=True) == (
        "No cover was removed.", ["b had no cover inside.", "Could not."])


def test_image_headers_are_read_as_metaflac_reads_them():
    def png(color, bits, extra=b""):
        ihdr = struct.pack(">II", 7, 5) + bytes([bits, color, 0, 0, 0])
        chunk = struct.pack(">I", 13) + b"IHDR" + ihdr + b"CRC!"
        return b"\x89PNG\r\n\x1a\n" + chunk + extra + struct.pack(">I", 0) + b"IEND" + b"CRC!"

    assert covers.image_info(png(2, 8)) == covers.PictureInfo("image/png", 7, 5, 24, 0)
    assert covers.image_info(png(6, 16)) == covers.PictureInfo("image/png", 7, 5, 64, 0)
    assert covers.image_info(png(0, 1)) == covers.PictureInfo("image/png", 7, 5, 1, 0)
    plte = struct.pack(">I", 12) + b"PLTE" + b"\x00" * 12 + b"CRC!"
    assert covers.image_info(png(3, 4, plte)) == covers.PictureInfo("image/png", 7, 5, 24, 4)
    assert covers.image_info(png(3, 4)) is None, "an indexed PNG without its palette"
    sof2 = b"\xff\xc2\x00\x11\x08\x02\x58\x03\x20\x03" + b"\x00" * 9
    jpeg = b"\xff\xd8\xff\xe0\x00\x10JFIF\x00" + b"\x00" * 9 + b"\xff\xff" + sof2 + b"\xff\xda"
    assert covers.image_info(jpeg) == covers.PictureInfo("image/jpeg", 800, 600, 24, 0)
    assert covers.image_info(b"\xff\xd8\xff\xda\x00\x02") is None and covers.image_info(b"nope") is None
    assert [covers.image_format(x) for x in (jpeg, png(2, 8), b"GIF89a", b"BMxx", b"RIFF\0\0\0\0WEBPVP8 ", b"?")] == [
        "JPEG", "PNG", "GIF", "BMP", "WebP", ""]
    with pytest.raises(tagedit.TagError, match="not a JPEG or PNG"):
        tagedit.check_picture(b"GIF89a")
    with pytest.raises(tagedit.TagError, match="too large"):
        tagedit.check_picture(b"\xff\xd8" + b"\x00" * tagedit.MAX_PICTURE)
    assert tagedit.check_picture(jpeg, "image/JPG").width == 800
    assert covers.details_text(1200, 1200, "JPEG", 251_187) == "1,200 × 1,200 pixels · JPEG · 245.3 KB"
    assert covers.details_text(0, 0, "", 12) == "Unknown format · 12 bytes"


@needs_ffmpeg
def test_cover_source_tells_where_the_cover_comes_from(tmp_path):
    album = tmp_path / "Album"
    album.mkdir()
    a = make(album / "01 A.flac", ["-c:a", "flac"])
    b = make(album / "02 B.flac", ["-c:a", "flac"])
    assert covers.cover_source([a, b]) is None
    webp_free = picture(album / "Cover.JPG", "testsrc", "64x48")
    src = covers.cover_source([a, b])
    assert (src.kind, src.path, src.data, src.width) == ("folder", str(album / "Cover.JPG"), webp_free, 64)
    assert src.where == "Cover.JPG in the album's folder"
    png = picture(tmp_path / "p.png", "color=c=blue", "32x32")
    assert covers.folder_cover_name(str(album), png) == str(album / "cover.png")
    assert covers.folder_cover_name(str(album), webp_free) == str(album / "Cover.JPG"), "the spelling there is kept"
    saved = covers.save_folder_cover(str(album), webp_free)
    assert saved == str(album / "Cover.JPG") and sorted(os.listdir(album)) == ["01 A.flac", "02 B.flac", "Cover.JPG"]
    assert tagedit.set_cover([b], png)[0].ok
    src = covers.cover_source([a, b])
    assert (src.kind, src.path, src.format) == ("embedded", b, "PNG") and src.where == "Inside 02 B.flac"
    shown = covers.fit_png(webp_free, 32)
    big = covers.image_info(covers.fit_png(png, 500))
    assert (covers.image_info(shown).width, covers.image_info(shown).height) == (32, 24)
    assert (big.width, big.height) == (32, 32), "a picture is never enlarged"
