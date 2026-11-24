"""Synced lyrics from LRCLIB (https://lrclib.net), the way LRCGET does it.

Every song is looked up by the title, artist, album and length in its tags.
Synced lyrics are saved as ``<name>.lrc`` next to the song, which most music
players show line by line in time with the music; when LRCLIB only has plain
(unsynced) lyrics they are saved as ``<name>.txt``, and a song LRCLIB lists as
instrumental gets an ``.lrc`` holding ``[au: instrumental]``, so it is not
looked up again.  These are the conventions of LRCGET
(https://github.com/tranxuanthang/lrcget, the official LRCLIB client), so
files made by either program are understood by the other.

LRCLIB is a free service and asks batch clients to behave: requests go out one
at a time, with a pause between them, the program names itself in the
User-Agent, and a 429/503 answer's Retry-After is always honoured (with some
jitter).  Only the standard library is used (urllib), so nothing new has to be
installed or bundled.

Lyrics files are never deleted: an existing ``.txt`` stays when a synced
``.lrc`` is added next to it, and existing files are only overwritten when the
user chose "Download again and replace them".

Credits: the lyrics come from LRCLIB and its contributors, and this feature is
modelled on LRCGET.  Both are by tranxuanthang and MIT-licensed (LRCGET:
"Copyright (c) 2023 tranxuanthang", https://github.com/tranxuanthang/lrcget;
the LRCLIB server: "Copyright (c) 2024 tranxuanthang",
https://github.com/tranxuanthang/lrclib).  This module was written for
Real-Hi-Res and contains no code from either; it follows LRCGET's conventions
and LRCLIB's published API.  CREDIT and ABOUT_CREDIT are shown in the desktop
app (the lyrics window and the About box).
"""

from __future__ import annotations

import http.client
import json
import os
import random
import re
import socket
import ssl
import tempfile
import threading
import time
import unicodedata
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass, field
from typing import Callable, Dict, Iterable, Iterator, List, Optional, Tuple

from . import APP_NAME, PROJECT_URL, __version__

DEFAULT_SERVER = "https://lrclib.net"
INSTRUMENTAL_LRC = "[au: instrumental]"   # LRCGET's mark for instrumental tracks
DURATION_TOLERANCE_S = 2.0   # LRCLIB matches a record whose length is within ±2 s
MAX_DURATION_S = 3600        # LRCLIB rejects longer durations; such songs are looked up without one
MIN_INTERVAL_S = 0.7         # between request starts: ~85/min, below the ~90/min of an identified client
MIN_GAP_S = 0.25             # pause after each answer (LRCLIB asks for 200-500 ms in batch work)
TIMEOUT_S = 15.0
MAX_ATTEMPTS = 4             # per request, for 429/503/5xx answers and network errors
MAX_BACKOFF_S = 8.0
MAX_RETRY_AFTER_S = 3600.0
MAX_LYRICS_BYTES = 512 * 1024  # a lyrics file read for display is cut here
INTERVAL_ENV = "REALHIRES_LYRICS_INTERVAL"  # tests only; never below MIN_INTERVAL_S for lrclib.net

SYNCED, PLAIN, INSTRUMENTAL = "synced", "plain", "instrumental"
PLAIN_ONLY, NOT_FOUND, SKIPPED, NO_TAGS, FAILED = "plain-only", "not-found", "skipped", "no-tags", "failed"
STATUS_LABEL = {
    SYNCED: "Synced lyrics saved",
    PLAIN: "Plain lyrics saved",
    INSTRUMENTAL: "Marked instrumental",
    PLAIN_ONLY: "Only plain lyrics",
    NOT_FOUND: "Not found",
    SKIPPED: "Skipped",
    NO_TAGS: "No title/artist",
    FAILED: "Failed",
}
SAVED_STATUSES = (SYNCED, PLAIN, INSTRUMENTAL)

# Shown in the lyrics window and the About box (the Rust version shows the same texts).
CREDIT = ("Lyrics are provided by LRCLIB (lrclib.net) and its contributors. This window works like LRCGET "
          "by tranxuanthang, the official LRCLIB client.")
ABOUT_CREDIT = ("Synced lyrics come from LRCLIB (lrclib.net) and its contributors. The lyrics feature is modelled "
                "on LRCGET by tranxuanthang (MIT License), the official LRCLIB client.")

EXISTING_CHOICES = ["Skip songs that have synced lyrics", "Skip songs that have any lyrics",
                    "Download again and replace them"]
EXISTING_CLI = dict(zip(("skip-synced", "skip-any", "replace"), EXISTING_CHOICES))


class LyricsError(Exception):
    """A lookup failed (network, server error, or an answer that could not be read)."""


class LyricsStopped(Exception):
    """The user stopped the downloads."""


def user_agent() -> str:
    """LRCLIB asks every client to name itself, its version and its home page."""
    return f"{APP_NAME} v{__version__} ({PROJECT_URL})"


def is_official(server: str) -> bool:
    host = (urllib.parse.urlsplit(server).hostname or "").rstrip(".").lower()
    return host == "lrclib.net" or host.endswith(".lrclib.net")


def default_interval(server: str) -> float:
    try:
        value = float(os.environ.get(INTERVAL_ENV, MIN_INTERVAL_S))
    except ValueError:
        value = MIN_INTERVAL_S
    value = max(0.0, value)
    return max(value, MIN_INTERVAL_S) if is_official(server) else value


# CA bundles of common systems.  A frozen build uses the OpenSSL it was built with, whose
# default paths are those of the build machine (a python.org framework path on macOS,
# /etc/pki on AlmaLinux), so on another system it may find no certificates at all.
_CA_FILES = (
    "/etc/ssl/certs/ca-certificates.crt",     # Debian, Ubuntu, Arch, Gentoo
    "/etc/pki/tls/certs/ca-bundle.crt",       # Fedora, RHEL, AlmaLinux
    "/etc/ssl/ca-bundle.pem",                 # openSUSE
    "/etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem",
    "/etc/ssl/cert.pem",                      # macOS, Alpine, BSD
    "/usr/local/etc/openssl/cert.pem",
    "/usr/local/etc/openssl@3/cert.pem",
    "/opt/homebrew/etc/openssl@3/cert.pem",
)
_SSL_CONTEXT: Optional[ssl.SSLContext] = None


def ssl_context() -> ssl.SSLContext:
    """A verifying TLS context that also works in the frozen builds."""
    global _SSL_CONTEXT
    if _SSL_CONTEXT is None:
        ctx = ssl.create_default_context()
        if not ctx.cert_store_stats().get("x509_ca"):
            for path in _CA_FILES:
                if os.path.isfile(path):
                    try:
                        ctx.load_verify_locations(cafile=path)
                        break
                    except (ssl.SSLError, OSError):
                        continue
        _SSL_CONTEXT = ctx
    return _SSL_CONTEXT


Transport = Callable[[str, Dict[str, str], float], Tuple[int, Dict[str, str], bytes]]


def _urlopen(url: str, headers: Dict[str, str], timeout: float) -> Tuple[int, Dict[str, str], bytes]:
    """GET `url`; returns (status, lower-case headers, body).  Network errors raise OSError."""
    req = urllib.request.Request(url, headers=headers)
    ctx = ssl_context() if url.lower().startswith("https:") else None
    try:
        with urllib.request.urlopen(req, timeout=timeout, context=ctx) as resp:
            return resp.status, {k.lower(): v for k, v in resp.headers.items()}, resp.read()
    except urllib.error.HTTPError as exc:  # 4xx/5xx still carry headers and a body
        try:
            body = exc.read()
        except (OSError, http.client.HTTPException):
            body = b""
        return exc.code, {k.lower(): v for k, v in (exc.headers or {}).items()}, body


def _network_reason(exc: BaseException) -> str:
    reason = getattr(exc, "reason", exc)
    if isinstance(reason, ssl.SSLCertVerificationError) or isinstance(exc, ssl.SSLCertVerificationError):
        return f"the certificate of the server could not be verified ({getattr(reason, 'verify_message', reason)})"
    if isinstance(reason, (socket.timeout, TimeoutError)):
        return "no answer in time"
    return str(reason) or type(reason).__name__


class LrclibClient:
    """Sequential, polite LRCLIB requests (one client per batch, used from one thread)."""

    def __init__(self, server: str = DEFAULT_SERVER, *, stop: Optional[threading.Event] = None,
                 transport: Optional[Transport] = None, min_interval: Optional[float] = None,
                 timeout: float = TIMEOUT_S, clock: Callable[[], float] = time.monotonic,
                 sleep: Callable[[float], None] = time.sleep, rng: Optional[random.Random] = None):
        self.server = (server or "").strip().rstrip("/") or DEFAULT_SERVER
        if "://" not in self.server:
            self.server = "https://" + self.server
        self.stop = stop
        self.transport = transport or _urlopen
        self.min_interval = default_interval(self.server) if min_interval is None else min_interval
        self.min_gap = min(MIN_GAP_S, self.min_interval)
        self.timeout = timeout
        self.clock = clock
        self._sleep = sleep
        self.rng = rng or random.Random()
        self._last_start: Optional[float] = None
        self._last_end: Optional[float] = None
        self._not_before = 0.0  # set by Retry-After; also holds for the next song after a failure
        self.requests = 0

    @property
    def host(self) -> str:
        return urllib.parse.urlsplit(self.server).netloc or self.server

    def _wait(self, seconds: float) -> None:
        if seconds <= 0:
            return
        if self.stop is not None:
            if self.stop.wait(seconds):
                raise LyricsStopped()
        else:
            self._sleep(seconds)

    def _throttle(self) -> None:
        if self.stop is not None and self.stop.is_set():
            raise LyricsStopped()
        due = self._not_before
        if self._last_start is not None:
            due = max(due, self._last_start + self.min_interval)
        if self._last_end is not None:
            due = max(due, self._last_end + self.min_gap)
        self._wait(due - self.clock())

    def _retry_after(self, headers: Dict[str, str]) -> Optional[float]:
        try:
            value = float(headers.get("retry-after", ""))
        except ValueError:
            return None
        return min(max(value, 0.0), MAX_RETRY_AFTER_S)

    def request(self, path: str, params: Dict[str, object]):
        """The parsed JSON answer, or None for 404.  Raises LyricsError / LyricsStopped."""
        url = f"{self.server}{path}?{urllib.parse.urlencode(params)}"
        headers = {"User-Agent": user_agent(), "Accept": "application/json"}
        backoff, problem = 1.0, ""
        for attempt in range(1, MAX_ATTEMPTS + 1):
            self._throttle()
            self._last_start = self.clock()
            self.requests += 1
            try:
                status, hdrs, body = self.transport(url, headers, self.timeout)
            except ValueError as exc:  # e.g. an address that is not http(s)://
                raise LyricsError(f"The LRCLIB server address {self.server!r} is not valid: {exc}.") from exc
            except (OSError, http.client.HTTPException) as exc:
                self._last_end = self.clock()
                if isinstance(getattr(exc, "reason", exc), ssl.SSLCertVerificationError):
                    raise LyricsError(f"Could not connect to {self.host} securely: {_network_reason(exc)}.") from exc
                problem, wait = f"could not reach {self.host}: {_network_reason(exc)}", backoff
            else:
                self._last_end = self.clock()
                if status == 200:
                    try:
                        return json.loads(body.decode("utf-8"))
                    except (UnicodeDecodeError, ValueError) as exc:
                        raise LyricsError(f"{self.host} sent an answer that could not be read.") from exc
                if status == 404:
                    return None
                if status in (429, 503):
                    wait = self._retry_after(hdrs)
                    if wait is None:
                        wait = 1.0 if status == 503 else backoff
                    wait += self.rng.uniform(0, 0.3 * wait)  # LRCLIB asks for up to 30 % jitter
                    self._not_before = self.clock() + wait
                    problem = ("too many requests, the server asked to wait" if status == 429
                               else "the server is busy")
                elif status >= 500:
                    problem, wait = f"server error (HTTP {status})", backoff
                else:
                    raise LyricsError(f"{self.host} refused the request (HTTP {status}{_api_message(body)}).")
            if attempt < MAX_ATTEMPTS:
                self._wait(wait)
                backoff = min(backoff * 2, MAX_BACKOFF_S)
        raise LyricsError(f"No answer from {self.host} after {MAX_ATTEMPTS} attempts ({problem}).")

    def get(self, track: "Track") -> Optional[dict]:
        rec = self.request("/api/get", track.params())
        return rec if isinstance(rec, dict) else None

    def search(self, track: "Track") -> List[dict]:
        params = {"track_name": track.title, "artist_name": track.primary_artist}
        found = self.request("/api/search", params)
        return [r for r in found if isinstance(r, dict)] if isinstance(found, list) else []


def _api_message(body: bytes) -> str:
    try:
        msg = json.loads(body.decode("utf-8")).get("message")
    except (UnicodeDecodeError, ValueError, AttributeError):
        return ""
    return f": {msg}" if isinstance(msg, str) and msg.strip() else ""


# ------------------------------------------------------------------ tracks ---
@dataclass
class Track:
    path: str
    title: str
    artist: str
    album: str = ""
    duration: Optional[float] = None

    @property
    def primary_artist(self) -> str:
        """FFmpeg joins several artist tags with ';' — the first one is the main artist."""
        return self.artist.split(";")[0].strip() or self.artist

    @property
    def label(self) -> str:
        return f"{self.artist} – {self.title}"

    def params(self) -> Dict[str, object]:
        p: Dict[str, object] = {"track_name": self.title,
                                "artist_name": ", ".join(a.strip() for a in self.artist.split(";") if a.strip())}
        if self.album:
            p["album_name"] = self.album
        if self.duration and 1 <= round(self.duration) <= MAX_DURATION_S:
            p["duration"] = int(round(self.duration))
        return p


def track_from_tags(path: str, tags: Optional[dict], duration: Optional[float]) -> Tuple[Optional[Track], str]:
    tags = tags or {}
    title = (tags.get("title") or "").strip()
    artist = (tags.get("artist") or tags.get("album_artist") or "").strip()
    if not title or not artist:
        missing = "title or artist tags" if not (title or artist) else "title tag" if not title else "artist tag"
        return None, f"The file has no {missing}, so LRCLIB cannot be asked for its lyrics."
    return Track(path, title, artist, (tags.get("album") or "").strip(), duration), ""


def read_track(path: str, result=None) -> Tuple[Optional[Track], str]:
    """The track to look up, from a scan result when there is one, else from the file's tags.
    Raises LyricsError when the file cannot be read."""
    if result is not None and result.tags and result.duration:
        return track_from_tags(path, result.tags, result.duration)
    from .probe import probe

    try:
        info = probe(path)
    except Exception as exc:  # noqa: BLE001 - any unreadable file becomes a message
        raise LyricsError(f"The tags could not be read: {exc}") from exc
    return track_from_tags(path, info.tags, info.duration)


# ------------------------------------------------------------------ lyrics ---
@dataclass
class Lyrics:
    synced: str = ""
    plain: str = ""
    instrumental: bool = False
    lrclib_id: Optional[int] = None
    title: str = ""
    artist: str = ""
    album: str = ""
    duration: Optional[float] = None
    how: str = "exact"   # "exact" (/api/get) or "search" (/api/search)


def _text(value) -> str:
    return value if isinstance(value, str) and value.strip() else ""


def _num(value) -> Optional[float]:
    try:
        return float(value)
    except (TypeError, ValueError):
        return None


def parse_record(rec, how: str = "exact") -> Optional[Lyrics]:
    """A usable LRCLIB record as Lyrics (synced > plain > instrumental, as LRCGET), else None."""
    if not isinstance(rec, dict):
        return None
    synced, plain = _text(rec.get("syncedLyrics")), _text(rec.get("plainLyrics"))
    instrumental = bool(rec.get("instrumental")) or is_instrumental_lrc(synced)
    if instrumental and not plain and (not synced or is_instrumental_lrc(synced)):
        synced = ""
    elif synced or plain:
        instrumental = False
    if not (synced or plain or instrumental):
        return None
    if synced and not plain:
        plain = strip_timestamps(synced)
    rid = rec.get("id")
    return Lyrics(synced, plain, instrumental, rid if isinstance(rid, int) else None,
                  _text(rec.get("trackName") or rec.get("name")), _text(rec.get("artistName")),
                  _text(rec.get("albumName")), _num(rec.get("duration")), how)


def is_instrumental_lrc(text: str) -> bool:
    low = (text or "").lower()
    return "[au:" in low and "instrumental" in low


_TIMESTAMP = re.compile(r"^(\s*\[[^\]]*\])+\s*")


def strip_timestamps(synced: str) -> str:
    return "\n".join(_TIMESTAMP.sub("", line) for line in synced.splitlines()).strip()


_BRACKETS = re.compile(r"[(\[（【].*?[)\]）】]")
_FEAT = re.compile(r"\s(feat|ft|featuring)\.?\s.*$")
_VERSION = re.compile(r"\s[-–]\s.*\b(remaster(ed)?|version|edit|mix|live|mono|stereo|demo|bonus)\b.*$")


def normalize_name(name: str) -> str:
    """Comparable form of a title or artist: case, punctuation, '(Remastered 2011)' and
    'feat. X' parts do not matter.  Letters of every script are kept."""
    base = unicodedata.normalize("NFKC", name or "").casefold()
    for s in (_VERSION.sub(" ", _FEAT.sub(" ", _BRACKETS.sub(" ", base))), base):
        words = re.sub(r"[\W_]+", " ", s).split()
        if words:
            return " ".join(words)
    return base.strip()


def _same_artist(a: str, b: str) -> bool:
    na, nb = normalize_name(a), normalize_name(b)
    if not na or not nb:
        return False
    return na == nb or f" {na} " in f" {nb} " or f" {nb} " in f" {na} "


def best_match(records: Iterable[dict], track: Track) -> Optional[Lyrics]:
    """The search result that is this song: same title and artist and, when the song's
    length is known, a length within ±2 s (as /api/get requires).  Synced lyrics win,
    then the closest length, then the same album."""
    want_title = normalize_name(track.title)
    best, best_key = None, None
    for rec in records:
        lyr = parse_record(rec, how="search")
        if lyr is None or normalize_name(lyr.title) != want_title:
            continue
        if not _same_artist(lyr.artist, track.primary_artist):
            continue
        album_ok = bool(track.album) and normalize_name(lyr.album) == normalize_name(track.album)
        if track.duration:
            if lyr.duration is None or abs(lyr.duration - track.duration) > DURATION_TOLERANCE_S:
                continue
            diff = abs(lyr.duration - track.duration)
        else:
            if track.album and not album_ok:
                continue  # without a length, only the same album makes it safe
            diff = 0.0
        key = (0 if lyr.synced else 1 if lyr.plain else 2, diff, 0 if album_ok else 1)
        if best_key is None or key < best_key:
            best, best_key = lyr, key
    return best


def find_lyrics(client: LrclibClient, track: Track, search: bool = True) -> Optional[Lyrics]:
    """/api/get (exact title, artist, album and length), then optionally /api/search."""
    lyr = parse_record(client.get(track))
    if lyr is not None and (lyr.synced or lyr.instrumental or not search):
        return lyr  # only plain lyrics: a search may still find a synced copy of the same song
    if search:
        found = best_match(client.search(track), track)
        if found is not None and (lyr is None or (found.synced and not lyr.synced)):
            return found
    return lyr


# ------------------------------------------------------------------- files ---
def sidecar(path: str, ext: str) -> str:
    """<folder>/<name>.lrc for <folder>/<name>.flac."""
    return os.path.splitext(path)[0] + ext


def existing_lyrics(path: str) -> Tuple[Optional[str], Optional[str]]:
    """(the .lrc, the .txt) next to a song, each None when it does not exist."""
    lrc, txt = sidecar(path, ".lrc"), sidecar(path, ".txt")
    return (lrc if os.path.isfile(lrc) else None), (txt if os.path.isfile(txt) else None)


def _replace_mode(cfg) -> bool:
    return cfg.lyrics_existing == EXISTING_CHOICES[2]


def skip_reason(path: str, cfg) -> str:
    """Why a song is not looked up at all ('' = look it up), per Settings → Lyrics."""
    if _replace_mode(cfg):
        return ""
    lrc, txt = existing_lyrics(path)
    if lrc:
        with_what = "an instrumental mark" if is_instrumental_lrc(_read_head(lrc)) else "synced lyrics"
        return f"Already has {with_what} ({os.path.basename(lrc)})."
    mode = save_mode(cfg)
    if mode != "lrc":
        from . import embed

        synced, plain = embed.embedded_lyrics(path)
        text = synced or plain
        if text and is_instrumental_lrc(text):
            return "Already has an instrumental mark inside the file."
        if synced:
            return "Already has synced lyrics inside the file."
        if plain and (mode == "plain" or cfg.lyrics_existing == EXISTING_CHOICES[1]):
            return "Already has lyrics inside the file."
    if txt and cfg.lyrics_existing == EXISTING_CHOICES[1]:
        return f"Already has lyrics ({os.path.basename(txt)})."
    return ""


def _read_head(path: str, size: int = 4096) -> str:
    try:
        with open(path, "rb") as fh:
            return fh.read(size).decode("utf-8", "replace")
    except OSError:
        return ""


def write_text(path: str, text: str) -> None:
    """Write UTF-8 text with '\\n' line ends through a temporary file, so a song never
    ends up with half a lyrics file."""
    data = ("\n".join(line.rstrip() for line in text.replace("\r\n", "\n").replace("\r", "\n").split("\n"))
            .strip("\n") + "\n").encode("utf-8")
    folder = os.path.dirname(os.path.abspath(path))
    fd, tmp = tempfile.mkstemp(prefix=".realhires-lyrics-", suffix=".tmp", dir=folder)
    try:
        with os.fdopen(fd, "wb") as fh:
            fh.write(data)
        os.replace(tmp, path)
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


def read_lyrics_file(path: str) -> Optional[Tuple[str, str, str]]:
    """(kind, text, file) of a song's lyrics, for display; kind is synced, instrumental or
    plain.  None when there is none.  Synced lyrics win wherever they are: the .lrc next
    to the song, then the lyrics inside the file (MP3, FLAC) when they are synced, then
    the .txt next to it, then plain lyrics inside the file.  (LRCGET's "plain text" and
    "embed" exports together leave a .txt beside a file whose tag holds synced lyrics.)
    A .txt or plain tag that holds LRC (`embed.looks_synced`) counts as synced."""
    from . import embed  # 2.0: lyrics inside the file (MP3, FLAC)

    lrc, txt = existing_lyrics(path)

    def read(file: str) -> Optional[str]:
        try:
            with open(file, "rb") as fh:
                return fh.read(MAX_LYRICS_BYTES).decode("utf-8-sig", "replace")
        except OSError:
            return None

    text = read(lrc) if lrc else None
    if text is not None:
        return (INSTRUMENTAL if is_instrumental_lrc(text) else SYNCED), text, lrc
    inside = embed.lyrics_of(path)
    if inside is not None and inside[0] != PLAIN:
        return inside[0], inside[1], path
    text = read(txt) if txt else None
    if text is not None:
        if is_instrumental_lrc(text):
            return INSTRUMENTAL, text, txt
        return (SYNCED if embed.looks_synced(text) else PLAIN), text, txt
    if inside is not None:
        return inside[0], inside[1], path
    return None


@dataclass
class LyricsOutcome:
    path: str
    status: str
    message: str                      # a full sentence, shown verbatim
    track: Optional[Track] = None
    written: List[str] = field(default_factory=list)
    how: str = ""                     # "exact" or "search" when lyrics were found

    @property
    def saved(self) -> bool:
        return self.status in SAVED_STATUSES and bool(self.written)


def short_status(o: LyricsOutcome) -> str:
    """A few words for a table cell (the GUI); `message` holds the full sentence."""
    if o.status in SAVED_STATUSES and not o.written and o.message.startswith("Would"):
        return "Would be saved (dry run)"
    if o.status == SYNCED:
        return "Synced lyrics saved" + (", found by searching" if o.how == "search" else "")
    if o.status == PLAIN:
        return "Plain lyrics saved (no synced ones)"
    if o.status == INSTRUMENTAL:
        return "Marked instrumental" if o.written else "Instrumental, not marked"
    if o.status == SKIPPED:
        return short_skip(o.message)
    if o.status == FAILED:
        return "Failed: " + o.message
    return {PLAIN_ONLY: "Only plain lyrics, not saved", NOT_FOUND: "Not found on LRCLIB",
            NO_TAGS: "No title or artist tag"}.get(o.status, STATUS_LABEL.get(o.status, o.status))


def short_skip(reason: str) -> str:
    """'Already has synced lyrics (Song.lrc).' -> 'Already has synced lyrics'."""
    if reason.startswith("Already"):
        return reason.split(" (")[0].rstrip(".")
    return "Kept the existing .txt"


def save_mode(cfg) -> str:
    """"lrc", "embed", "both" or "plain" from Settings → Lyrics → Save lyrics as (2.0)."""
    from .settings import LYRICS_SAVE_CHOICES

    return {LYRICS_SAVE_CHOICES[1]: "embed", LYRICS_SAVE_CHOICES[2]: "both",
            LYRICS_SAVE_CHOICES[3]: "plain"}.get(getattr(cfg, "lyrics_save_as", ""), "lrc")


def save_lyrics(path: str, lyr: Lyrics, cfg, track: Optional[Track] = None, dry_run: bool = False) -> LyricsOutcome:
    from . import embed

    replace = _replace_mode(cfg)
    lrc, txt = sidecar(path, ".lrc"), sidecar(path, ".txt")
    lrc_name, txt_name, name = os.path.basename(lrc), os.path.basename(txt), os.path.basename(path)
    via = " (found by searching)" if lyr.how == "search" else ""
    mode, note = save_mode(cfg), ""
    if mode != "lrc" and not embed.supported(path):
        mode, note = "lrc", " Lyrics can only be embedded in MP3 and FLAC files."
    files: List[Tuple[str, str]] = []      # (file, text) to write next to the song
    inside: Optional[Tuple[str, str]] = None  # (synced, plain) to embed in the song
    if lyr.synced:
        plain = lyr.plain or strip_timestamps(lyr.synced)
        if mode == "plain":
            status, inside = PLAIN, ("", plain)
            msg = f"Embedded the lyrics in {name}, without timing{via}."
        else:
            status = SYNCED
            if mode in ("lrc", "both"):
                files.append((lrc, lyr.synced))
            if mode in ("embed", "both"):
                inside = (lyr.synced, plain)
            msg = {"lrc": f"Saved synced lyrics as {lrc_name}{via}.",
                   "embed": f"Embedded synced lyrics in {name}{via}.",
                   "both": f"Saved synced lyrics as {lrc_name} and embedded them in {name}{via}."}[mode]
    elif lyr.plain:
        if not cfg.lyrics_plain:
            what = "saving them as .txt" if mode == "lrc" else "saving plain lyrics"
            return LyricsOutcome(path, PLAIN_ONLY, "LRCLIB has only plain (unsynced) lyrics for this song, and "
                                 f"{what} is turned off.", track, how=lyr.how)
        keep_txt = os.path.isfile(txt) and not replace
        if mode == "lrc" and keep_txt:
            return LyricsOutcome(path, SKIPPED, "LRCLIB has only plain lyrics for this song, so the existing "
                                 f"{txt_name} was kept.", track, how=lyr.how)
        status = PLAIN
        if mode == "lrc":
            files.append((txt, lyr.plain))
            msg = f"LRCLIB has only plain lyrics for this song; saved them as {txt_name}{via}."
        elif mode == "both" and not keep_txt:
            files.append((txt, lyr.plain))
            inside = ("", lyr.plain)
            msg = f"LRCLIB has only plain lyrics for this song; saved them as {txt_name} and embedded them in {name}{via}."
        else:
            inside = ("", lyr.plain)
            msg = f"LRCLIB has only plain lyrics for this song; embedded them in {name}{via}."
    else:
        if not cfg.lyrics_instrumental:
            return LyricsOutcome(path, INSTRUMENTAL, "LRCLIB lists this song as instrumental; marking "
                                 "instrumental songs is turned off, so nothing was saved.", track, how=lyr.how)
        if mode in ("lrc", "both") and os.path.isfile(txt) and not replace:
            return LyricsOutcome(path, SKIPPED, "LRCLIB lists this song as instrumental, but it has lyrics in "
                                 f"{txt_name}, which were kept.", track, how=lyr.how)
        status = INSTRUMENTAL
        if mode in ("lrc", "both"):
            files.append((lrc, INSTRUMENTAL_LRC))
        if mode in ("embed", "both"):
            inside = (INSTRUMENTAL_LRC, INSTRUMENTAL_LRC)
        elif mode == "plain":
            inside = ("", INSTRUMENTAL_LRC)
        msg = {"lrc": f"LRCLIB lists this song as instrumental; marked it in {lrc_name}{via}.",
               "both": f"LRCLIB lists this song as instrumental; marked it in {lrc_name} and in {name}{via}."}.get(
            mode, f"LRCLIB lists this song as instrumental; marked it in {name}{via}.")
    msg += note
    if dry_run:
        return LyricsOutcome(path, status, "Would save: " + msg[0].lower() + msg[1:], track, how=lyr.how)
    written = []
    for target, text in files:
        try:
            write_text(target, text)
        except OSError as exc:
            return LyricsOutcome(path, FAILED, f"Could not save {os.path.basename(target)}: {exc}", track,
                                 written, lyr.how)
        written.append(target)
    if inside is not None:
        try:
            before = os.stat(path)
            embed.embed_lyrics(path, *inside)
        except (OSError, embed.EmbedError) as exc:
            return LyricsOutcome(path, FAILED, f"Could not embed the lyrics in {name}: {exc}", track, written,
                                 lyr.how)
        written.append(path)
        _keep_saved_results(path, before, cfg)
    return LyricsOutcome(path, status, msg, track, written, lyr.how)


def _keep_saved_results(path: str, before: os.stat_result, cfg) -> None:
    """Writing lyrics into a file changes its size and time but not its audio,
    so its saved scan results stay valid (`ResultStore.retag()`)."""
    from . import store as store_mod
    st = store_mod.open_store(cfg)
    if st is None:
        return
    try:
        st.retag(path, before.st_size, before.st_mtime_ns)
    finally:
        st.close()


def fetch_for_file(path: str, client: LrclibClient, cfg, result=None, dry_run: bool = False) -> LyricsOutcome:
    """Look one song up and save its lyrics.  Never raises, except LyricsStopped."""
    why = skip_reason(path, cfg)
    if why:
        return LyricsOutcome(path, SKIPPED, why)
    track = None
    try:
        track, why = read_track(path, result)
        if track is None:
            return LyricsOutcome(path, NO_TAGS, why)
        lyr = find_lyrics(client, track, search=cfg.lyrics_search)
    except LyricsError as exc:
        return LyricsOutcome(path, FAILED, str(exc), track)
    if lyr is None:
        return LyricsOutcome(path, NOT_FOUND, "LRCLIB has no lyrics for this song (yet).", track)
    return save_lyrics(path, lyr, cfg, track, dry_run)


def run_lyrics(paths: Iterable[str], cfg, results: Optional[dict] = None, dry_run: bool = False,
               stop: Optional[threading.Event] = None,
               client: Optional[LrclibClient] = None) -> Iterator[LyricsOutcome]:
    """Outcomes of `paths`, one song at a time.  Stops quietly when `stop` is set."""
    client = client or LrclibClient(cfg.lyrics_server, stop=stop)
    results = results or {}
    for path in paths:
        if stop is not None and stop.is_set():
            return
        try:
            yield fetch_for_file(path, client, cfg, results.get(path), dry_run)
        except LyricsStopped:
            return


def lyrics_summary(outcomes: Iterable[LyricsOutcome]) -> str:
    """'12 with synced lyrics, 3 with plain lyrics, 5 not found, 20 skipped.'"""
    counts: Dict[str, int] = {}
    for o in outcomes:
        counts[o.status] = counts.get(o.status, 0) + 1
    words = [(SYNCED, "with synced lyrics"), (PLAIN, "with plain lyrics"),
             (INSTRUMENTAL, "instrumental"), (PLAIN_ONLY, "with only plain lyrics (not saved)"),
             (NOT_FOUND, "not found on LRCLIB"), (SKIPPED, "skipped (already had lyrics)"),
             (NO_TAGS, "without title/artist tags"), (FAILED, "failed")]
    parts = [f"{counts[k]} {w}" for k, w in words if counts.get(k)]
    return (", ".join(parts) + ".") if parts else "nothing to do."


def check_server(server: str, client: Optional[LrclibClient] = None) -> str:
    """One request to the LRCLIB server (CLI --check-lyrics-server).  Raises LyricsError."""
    client = client or LrclibClient(server)
    track = Track("", "I Want to Live", "Borislav Slavov", "Baldur's Gate 3 (Original Game Soundtrack)", 233)
    rec = client.get(track)
    found = "an answer with lyrics" if parse_record(rec) else "an answer"
    return f"{client.server} can be reached: the test lookup got {found}."
