"""Covers from MusicBrainz and the Cover Art Archive (2.5, docs/player-spec.md, "2.5", item 9).

Search Cover Online → MusicBrainz… looks an album up on MusicBrainz (the existing
client: one request a second, the app's User-Agent) and shows the front covers of up
to `MAX_RELEASES` matching releases from the Cover Art Archive
(https://coverartarchive.org, run by the Internet Archive and MusicBrainz):
``/release/<mbid>/front-250`` for the grid and ``/release/<mbid>/front`` for the one
the user chose.

The Cover Art Archive gets the same care as MusicBrainz: `CoverArtClient` sends one
download at a time (across the app, `_GATE`), names the app in the User-Agent, follows
the archive's redirects, reads at most `MAX_BYTES` and never tries a download twice;
when the archive asks to slow down (HTTP 429/503) nothing more is asked for
(`CoverArtBusy`).  Only the album's artist and title are sent (to MusicBrainz), and a
release's id (to the archive); never a path or a file name.  The address is
`DEFAULT_SERVER`, or `$REALHIRES_COVERART` for tests, which must never contact
the real archive (tests/fake_coverart.py).

The pictures belong to their owners; the archive only stores them.  This module was
written for Universal Music Player against the archive's published API
(https://musicbrainz.org/doc/Cover_Art_Archive/API) and contains no code from
MusicBrainz or Picard.  The Rust version (rust/crates/core/src/coverart.rs) builds the
same requests and gives the same texts (rust/crates/core/tests/python_coverart.json).
"""

from __future__ import annotations

import http.client
import os
import ssl
import threading
import urllib.error
import urllib.parse
import urllib.request
from typing import Callable, Dict, List, Optional, Tuple

from . import covers
from .lyrics import _network_reason, ssl_context
from .musicbrainz import MusicBrainzClient, Release, user_agent

DEFAULT_SERVER = "https://coverartarchive.org"
SERVER_ENV = "REALHIRES_COVERART"   # tests point it at a fake archive on 127.0.0.1
MAX_BYTES = 20 * 1024 * 1024        # a picture larger than this is not taken
MAX_RELEASES = 12                   # releases asked of MusicBrainz, and covers shown
THUMB_PX = 250                      # the grid's pictures: /front-250
TIMEOUT_S = 30.0

CREDIT = ("Covers come from the Cover Art Archive (coverartarchive.org), found through MusicBrainz "
          "(musicbrainz.org). The pictures belong to their owners.")
ABOUT_CREDIT = "Cover search: MusicBrainz and the Cover Art Archive (coverartarchive.org)."

# (status, lower-case headers, body); the body is at most `limit` + 1 bytes long
Download = Callable[[str, Dict[str, str], float, int], Tuple[int, Dict[str, str], bytes]]


class CoverArtError(Exception):
    """A picture could not be had (the message is a full sentence)."""


class CoverArtBusy(CoverArtError):
    """The archive asked to slow down: nothing more is asked for now."""


class CoverArtStopped(Exception):
    """The window was closed."""


def server() -> str:
    """The archive's address: `$REALHIRES_COVERART` (tests), else DEFAULT_SERVER."""
    return normalize_server(os.environ.get(SERVER_ENV, ""))


def normalize_server(address: str) -> str:
    s = (address or "").strip().rstrip("/") or DEFAULT_SERVER
    return s if "://" in s else "https://" + s


def front_url(mbid: str, thumb: bool = True, base: Optional[str] = None) -> str:
    """Where a release's front cover is: the 250-pixel one for the grid, or as it was uploaded."""
    base = normalize_server(base) if base else server()
    return f"{base}/release/{urllib.parse.quote(mbid, safe='')}/{'front-' + str(THUMB_PX) if thumb else 'front'}"


def search_terms(album_artist: str = "", artist: str = "", album: str = "", title: str = "") -> Tuple[str, str]:
    """(release, artist) that MusicBrainz is asked for: the album and its artist (the album
    artist, else the artist); a song without an album: its title and artist."""
    album = covers._part(album)
    if album:
        return album, covers._part(album_artist) or covers._part(artist)
    return covers._part(title), covers._part(artist) or covers._part(album_artist)


def find_releases(client: MusicBrainzClient, album: str, artist: str = "") -> List[Release]:
    """Up to MAX_RELEASES releases called `album` (by `artist`), as MusicBrainz ranks them,
    each once.  Raises MusicBrainzError / MusicBrainzStopped."""
    out: List[Release] = []
    seen = set()
    for r in client.search_releases(album, artist, MAX_RELEASES):
        if r.id and r.id not in seen:
            seen.add(r.id)
            out.append(r)
    return out[:MAX_RELEASES]


def release_facts(r: Release) -> str:
    """The grey line under a cover: the year, the country and the format ("2001 · JP · CD")."""
    return " · ".join(x for x in (r.date[:4], r.country, r.formats) if x)


def _download(url: str, headers: Dict[str, str], timeout: float, limit: int) -> Tuple[int, Dict[str, str], bytes]:
    """GET `url`, following redirects (the archive answers with one to the Internet
    Archive); at most `limit` + 1 bytes of the body are read.  Network errors raise OSError."""
    req = urllib.request.Request(url, headers=headers)
    ctx = ssl_context() if url.lower().startswith("https:") else None
    try:
        with urllib.request.urlopen(req, timeout=timeout, context=ctx) as resp:
            return resp.status, {k.lower(): v for k, v in resp.headers.items()}, resp.read(limit + 1)
    except urllib.error.HTTPError as exc:  # 4xx/5xx: the body is not needed
        return exc.code, {k.lower(): v for k, v in (exc.headers or {}).items()}, b""


# One download at a time across the app, whatever window asks.
_GATE = threading.Lock()


def megabytes(n: int) -> str:
    return f"{n // (1024 * 1024)} MB"


class CoverArtClient:
    """Front covers from the Cover Art Archive, one download at a time, each tried once."""

    def __init__(self, base: Optional[str] = None, *, stop: Optional[threading.Event] = None,
                 download: Optional[Download] = None, timeout: float = TIMEOUT_S, max_bytes: int = MAX_BYTES):
        self.server = normalize_server(base) if base else server()
        self.stop = stop
        self.download = download or _download
        self.timeout = timeout
        self.max_bytes = max_bytes
        self.requests = 0

    @property
    def host(self) -> str:
        return urllib.parse.urlsplit(self.server).netloc or self.server

    def front(self, mbid: str, thumb: bool = True) -> Optional[bytes]:
        """The front cover of a release (JPEG, PNG...), or None when the archive has none.
        Raises CoverArtError (CoverArtBusy when the archive asked to slow down) / CoverArtStopped."""
        url = front_url(mbid, thumb, self.server)
        headers = {"User-Agent": user_agent(), "Accept": "image/*"}
        with _GATE:
            if self.stop is not None and self.stop.is_set():
                raise CoverArtStopped()
            self.requests += 1
            try:
                status, _, body = self.download(url, headers, self.timeout, self.max_bytes)
            except ValueError as exc:
                raise CoverArtError(f"The Cover Art Archive address {self.server!r} is not valid: {exc}.") from exc
            except (OSError, http.client.HTTPException) as exc:
                if isinstance(getattr(exc, "reason", exc), ssl.SSLCertVerificationError):
                    raise CoverArtError(f"Could not connect to {self.host} securely: {_network_reason(exc)}.") from exc
                raise CoverArtError(f"Could not reach {self.host}: {_network_reason(exc)}.") from exc
        if status == 404:
            return None
        if status in (429, 503):
            raise CoverArtBusy(f"{self.host} is busy and asked to slow down. Try again in a few minutes.")
        if status != 200:
            raise CoverArtError(f"{self.host} could not send the picture (HTTP {status}).")
        if len(body) > self.max_bytes:
            raise CoverArtError(f"The picture is larger than {megabytes(self.max_bytes)} and was not downloaded.")
        if not covers.image_format(body):
            raise CoverArtError(f"{self.host} sent something that is not a picture.")
        return body


__all__ = ["DEFAULT_SERVER", "SERVER_ENV", "MAX_BYTES", "MAX_RELEASES", "THUMB_PX", "TIMEOUT_S", "CREDIT",
           "ABOUT_CREDIT", "CoverArtError", "CoverArtBusy", "CoverArtStopped", "server", "normalize_server",
           "front_url", "search_terms", "find_releases", "release_facts", "megabytes", "CoverArtClient"]
