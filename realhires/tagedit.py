"""Editing a song's information (2.0): title, artist, album, album artist,
genre, release date, track, disc and composer.

FLAC and MP3 files are written natively, as the embedded lyrics are
(`embed.flac_rewrite()`, `embed.mp3_rewrite()`): only the Vorbis comment block
or the ID3v2 tag changes, the audio frames are copied byte for byte, and the
new file replaces the old one through a temporary file.  Other formats (M4A
with AAC or ALAC, Ogg Vorbis, Opus, WAV, AIFF, WavPack, WMA...) are copied by
FFmpeg with ``-c copy`` into a new container that carries the new tags; the
audio packets stay the same, and the copy must have the original's sample
rate, channels and length before it replaces the file.  Only the fields that
were changed are written, an empty value removes a field, and every other tag
and the cover art are kept.

After a file has been written, its saved quality-check results are moved to
its new size and time with the new tags (`ResultStore.retag()`), because its
audio did not change, and the library reads it again.

The Rust version (`rust/crates/core/src/tagedit.rs`) writes the same bytes and
runs the same FFmpeg command; `rust/tools/gen_player_vectors.py` writes the
Python answers it is tested against.
"""

from __future__ import annotations

import os
import re
import subprocess
import tempfile
from dataclasses import dataclass, field
from typing import Callable, Dict, Iterable, List, Optional, Tuple

from . import embed

FIELDS = ("title", "artist", "album", "album_artist", "genre", "date", "track", "disc", "composer")
LABELS = {"title": "Title", "artist": "Artist", "album": "Album", "album_artist": "Album artist",
          "genre": "Genre", "date": "Release date", "track": "Track", "disc": "Disc", "composer": "Composer"}
HINTS = {"date": "a year (2004) or a date (2004-05-28)", "track": "3, or 3/12 with the number of tracks",
         "disc": "1, or 1/2 with the number of discs"}
MAX_VALUE = 1000
NATIVE = (".flac", ".mp3")
STREAM_TAGS = (".ogg", ".oga", ".opus", ".spx")   # Ogg keeps the tags with the audio stream
ID3_CHUNK = (".aif", ".aiff", ".aifc")            # AIFF: an ID3 chunk holds every field
NO_MUXER = (".ape", ".dsf", ".dff", ".tak", ".mpc", ".ofr", ".tta")  # FFmpeg reads these but cannot write them

_FLAC = {
    "title": ("TITLE", ("TITLE",)),
    "artist": ("ARTIST", ("ARTIST",)),
    "album": ("ALBUM", ("ALBUM",)),
    "album_artist": ("ALBUMARTIST", ("ALBUMARTIST", "ALBUM ARTIST", "ALBUM_ARTIST")),
    "genre": ("GENRE", ("GENRE",)),
    "date": ("DATE", ("DATE", "YEAR")),
    "track": ("TRACKNUMBER", ("TRACKNUMBER", "TRACK")),
    "disc": ("DISCNUMBER", ("DISCNUMBER", "DISC")),
    "composer": ("COMPOSER", ("COMPOSER",)),
}
_FLAC_TOTAL = {"track": ("TRACKTOTAL", ("TRACKTOTAL", "TOTALTRACKS")),
               "disc": ("DISCTOTAL", ("DISCTOTAL", "TOTALDISCS"))}
_ID3 = {"title": b"TIT2", "artist": b"TPE1", "album": b"TALB", "album_artist": b"TPE2", "genre": b"TCON",
        "track": b"TRCK", "disc": b"TPOS", "composer": b"TCOM"}
_NUMBER = re.compile(r"^([0-9]+)(?:/([0-9]+))?$")
_YEAR = re.compile(r"^[0-9]{4}")
_FULL_DATE = re.compile(r"^[0-9]{4}-([0-9]{2})-([0-9]{2})")


class TagError(Exception):
    """The tags could not be changed; the message is a full sentence."""


def clean_value(value) -> str:
    """One line: surrounding space removed, runs of white space (newlines too) made one space."""
    return " ".join(str(value or "").split())


def split_number(value: str) -> Tuple[str, str]:
    """("3", "12") for "3/12", ("3", "") for "3", ("", "") for ""; the value must be checked."""
    m = _NUMBER.match(value.replace(" ", ""))
    if not m:
        return "", ""
    return str(int(m.group(1))), (str(int(m.group(2))) if m.group(2) else "")


def check_changes(changes: Dict[str, str]) -> Dict[str, str]:
    """The changes, cleaned (only known fields; track and disc as "3" or "3/12"), or
    TagError saying what is wrong with the first bad one."""
    out: Dict[str, str] = {}
    for f in FIELDS:
        if f not in changes:
            continue
        v = clean_value(changes[f])
        if len(v) > MAX_VALUE:
            raise TagError(f"The {LABELS[f].lower()} is too long: at most {MAX_VALUE:,} characters fit.")
        if v and f in ("track", "disc"):
            if not _NUMBER.match(v.replace(" ", "")):
                raise TagError(f"The {LABELS[f].lower()} must be a number, such as {HINTS[f]}.")
            n, total = split_number(v)
            v = f"{n}/{total}" if total else n
        if v and f == "date" and not _YEAR.match(v):
            raise TagError("Write the release date as a year (2004) or a date (2004-05-28).")
        out[f] = v
    return out


def common_values(tags: Iterable[Dict[str, str]]) -> Dict[str, Optional[str]]:
    """For the editor: each field's value when every song has the same one, None
    where the songs differ ("Several values")."""
    out: Dict[str, Optional[str]] = {}
    tags = list(tags)
    for f in FIELDS:
        values = {clean_value(t.get(f, "")) for t in tags}
        out[f] = values.pop() if len(values) == 1 else (None if values else "")
    return out


# ------------------------------------------------------------------ FLAC ---
def _put(items: list, key_of: Callable, removes: Iterable, new: list) -> list:
    """Replace the items whose key is in `removes` by `new`, at the place of the first
    one (or at the end when there is none)."""
    removes = set(removes)
    out, placed = [], False
    for it in items:
        if key_of(it) in removes:
            if not placed:
                out.extend(new)
                placed = True
        else:
            out.append(it)
    if not placed:
        out.extend(new)
    return out


def _comment(key: str, value: str) -> List[bytes]:
    return [f"{key}={value}".encode("utf-8")] if value else []


def flac_with_tags(data: bytes, changes: Dict[str, str]) -> bytes:
    """The FLAC file with the (checked) changes in its Vorbis comments."""
    def edit(comments):
        key_of = embed._comment_key
        for f in FIELDS:
            if f not in changes:
                continue
            value = changes[f]
            key, removes = _FLAC[f]
            if f in _FLAC_TOTAL:
                number, total = split_number(value)
                comments = _put(comments, key_of, removes, _comment(key, number))
                if total or not number:  # "3" keeps the number of tracks, "" removes it
                    tkey, tremoves = _FLAC_TOTAL[f]
                    comments = _put(comments, key_of, tremoves, _comment(tkey, total))
            else:
                comments = _put(comments, key_of, removes, _comment(key, value))
        return comments

    return embed.flac_rewrite(data, edit, "The tags are too long for a FLAC file.")


# ------------------------------------------------------------------- ID3 ---
def _text_frame(fid: bytes, text: str, major: int) -> tuple:
    enc, raw, _ = embed._enc(text, major)
    return fid, b"\x00\x00", bytes([enc]) + raw


def mp3_with_tags(data: bytes, changes: Dict[str, str]) -> bytes:
    """The MP3 file with the (checked) changes in its ID3v2 tag (a new ID3v2.4 tag
    when it has none).  ID3v2.3 keeps the release date as TYER (the year) and TDAT
    (day and month), ID3v2.4 as TDRC."""
    def edit(major, frames):
        key_of = lambda fr: fr[0]  # noqa: E731
        for f in FIELDS:
            if f not in changes:
                continue
            value = changes[f]
            if f == "date":
                new = []
                if major == 4:
                    if value:
                        new.append(_text_frame(b"TDRC", value, major))
                elif value:
                    new.append(_text_frame(b"TYER", value[:4], major))
                    m = _FULL_DATE.match(value)
                    if m:
                        new.append(_text_frame(b"TDAT", m.group(2) + m.group(1), major))
                frames = _put(frames, key_of, (b"TDRC", b"TYER", b"TDAT", b"TIME"), new)
            else:
                fid = _ID3[f]
                frames = _put(frames, key_of, (fid,), [_text_frame(fid, value, major)] if value else [])
        return frames

    try:
        return embed.mp3_rewrite(data, edit, "The tags are too long for an ID3 tag.")
    except embed.EmbedError as exc:
        raise TagError(str(exc)) from exc


# ---------------------------------------------------------------- FFmpeg ---
def ffmpeg_args(src: str, dst: str, changes: Dict[str, str]) -> List[str]:
    """The arguments (after the program) that copy `src` to `dst` with new tags."""
    ext = os.path.splitext(src)[1].lower()
    spec = "-metadata:s:a:0" if ext in STREAM_TAGS else "-metadata"
    args = ["-v", "error", "-nostdin", "-y", "-i", src, "-map", "0", "-c", "copy", "-map_metadata", "0",
            "-map_chapters", "0", "-fflags", "+bitexact"]
    for f in FIELDS:
        if f in changes:
            args += [spec, f"{f}={changes[f]}"]
    if ext in ID3_CHUNK:
        args += ["-write_id3v2", "1"]
    return args + [dst]


def cannot_edit(path: str) -> str:
    """Why the tags of this file cannot be changed, or ''."""
    ext = os.path.splitext(path)[1].lower()
    if ext in NO_MUXER:
        return (f"The information in {ext[1:].upper()} files cannot be changed here, because FFmpeg can read "
                f"them but not write them.")
    return ""


def _ffmpeg_copy(path: str, changes: Dict[str, str]) -> None:
    from .probe import _no_window, probe, tool

    why = cannot_edit(path)
    if why:
        raise TagError(why)
    ext = os.path.splitext(path)[1]
    folder = os.path.dirname(os.path.abspath(path))
    fd, tmp = tempfile.mkstemp(prefix=".realhires-tags-", suffix=ext, dir=folder)
    os.close(fd)
    try:
        proc = subprocess.run([tool("ffmpeg")] + ffmpeg_args(path, tmp, changes), capture_output=True,
                              stdin=subprocess.DEVNULL, **_no_window())
        if proc.returncode != 0:
            msg = proc.stderr.decode("utf-8", "replace").strip().splitlines()
            raise TagError("FFmpeg could not copy the file with the new information: "
                           f"{msg[-1] if msg else f'exit code {proc.returncode}'}.")
        before, after = probe(path), probe(tmp)
        if (after.sample_rate != before.sample_rate or after.channels != before.channels
                or abs((after.duration or 0) - (before.duration or 0)) > max(0.5, 0.01 * (before.duration or 0))):
            raise TagError("FFmpeg's copy did not have the same audio as the original, so the file was left "
                           "unchanged.")
        try:
            os.chmod(tmp, os.stat(path).st_mode & 0o7777)
        except OSError:
            pass
        os.replace(tmp, path)
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


def write_tags(path: str, changes: Dict[str, str]) -> str:
    """Write the (checked) changes into the file; returns "native" or "ffmpeg".
    Raises TagError or OSError, and leaves the file as it was then."""
    if not changes:
        return "native"
    ext = os.path.splitext(path)[1].lower()
    if ext in NATIVE:
        with open(path, "rb") as fh:
            data = fh.read()
        try:
            new = flac_with_tags(data, changes) if ext == ".flac" else mp3_with_tags(data, changes)
        except TagError:
            if ext != ".mp3":
                raise
            _ffmpeg_copy(path, changes)  # ID3v2.2 or unsynchronised tags: FFmpeg writes a new one
            return "ffmpeg"
        except embed.EmbedError as exc:
            raise TagError(str(exc)) from exc
        embed.replace_file(path, new)
        return "native"
    _ffmpeg_copy(path, changes)
    return "ffmpeg"


# ------------------------------------------------------- saving songs ---
@dataclass
class TagOutcome:
    path: str
    ok: bool
    message: str = ""                       # a full sentence when it failed or was kept only partly
    tags: Dict[str, str] = field(default_factory=dict)   # what the file holds now (normalised)


def same_value(f: str, wanted: str, got: str) -> bool:
    if f in ("track", "disc"):
        return split_number(wanted)[0] == split_number(got)[0]
    return clean_value(wanted) == clean_value(got)


def _join(words: List[str]) -> str:
    return words[0] if len(words) == 1 else ", ".join(words[:-1]) + " and " + words[-1]


def kept_note(name: str, changes: Dict[str, str], tags: Dict[str, str]) -> str:
    """'' when the file holds every change, else a sentence saying what its format
    could not keep."""
    lost = [f"the {LABELS[f].lower()}" for f in FIELDS if f in changes and changes[f] and not tags.get(f)]
    other = [f"the {LABELS[f].lower()} reads “{tags[f]}”" for f in FIELDS
             if f in changes and tags.get(f) and not same_value(f, changes[f], tags[f])]
    parts = []
    if lost:
        parts.append(f"its format has no place for {_join(lost)}")
    if other:
        parts.append(_join(other) + ", as far as its format allows")
    return f"{name} was saved, but {'; '.join(parts)}." if parts else ""


def save_tags(paths: List[str], changes: Dict[str, str], cfg=None,
              progress: Optional[Callable[[int, int, str], None]] = None) -> List[TagOutcome]:
    """Write the changes into every file, keep their saved results and update the
    library.  Never raises; one outcome per file."""
    from . import musiclib, store as store_mod
    from .probe import probe

    try:
        changes = check_changes(changes)
    except TagError as exc:
        return [TagOutcome(p, False, str(exc)) for p in paths]
    results = []
    st = store_mod.open_store(cfg) if cfg is not None and getattr(cfg, "save_results", True) else None
    lib = None
    try:
        lib = musiclib.Library()
    except Exception:  # noqa: BLE001
        lib = None
    try:
        for i, path in enumerate(paths):
            if progress is not None:
                progress(i, len(paths), path)
            name = os.path.basename(path)
            try:
                before = os.stat(path)
                write_tags(path, changes)
            except (TagError, OSError) as exc:
                results.append(TagOutcome(path, False, f"Could not save the information of {name}: {exc}"))
                continue
            try:
                tags = probe(path).tags or {}
            except Exception:  # noqa: BLE001
                tags = {}
            if st is not None:
                st.retag(path, before.st_size, before.st_mtime_ns, tags=tags)
            if lib is not None:
                lib.update_file(path)
            results.append(TagOutcome(path, True, kept_note(name, changes, tags), tags))
        if progress is not None:
            progress(len(paths), len(paths), "")
    finally:
        if st is not None:
            st.close()
        if lib is not None:
            lib.close()
    return results


def save_summary(outcomes: List[TagOutcome]) -> Tuple[str, List[str]]:
    """(headline, sentences) for the editor after saving."""
    ok = [o for o in outcomes if o.ok]
    bad = [o for o in outcomes if not o.ok]
    n = len(ok)
    head = (f"Saved the information of {n:,} song{'s' if n != 1 else ''}." if ok and not bad else
            f"Saved {n:,} of {len(outcomes):,} songs." if ok else
            "Nothing was saved.")
    lines: List[str] = []
    for o in ok + bad:
        if o.message and o.message not in lines:
            lines.append(o.message)
    return head, lines
