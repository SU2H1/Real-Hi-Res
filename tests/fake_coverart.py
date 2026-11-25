"""A small invented Cover Art Archive (2.5): front covers for some releases of
tests/fake_musicbrainz.py's catalogue, served like https://coverartarchive.org.

Tests must never contact coverartarchive.org.  `FakeCoverArt` answers
`/release/<mbid>/front-250` and `/release/<mbid>/front` with a redirect (307, as the
archive redirects to the Internet Archive) to `/img/<name>.png`, which is the picture;
a release without a cover is 404.  It records every request with its User-Agent, and
how many requests were being answered at the same time (`max_active` must stay 1: the
app downloads one picture at a time).  Every picture here is made from numbers.
"""

from __future__ import annotations

import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Dict, List, Optional, Tuple

import numpy as np

from realhires.imaging import to_png

# release id -> the colours of its cover (left half, right half); releases not here have none
COVERS: Dict[str, Tuple[Tuple[int, int, int], Tuple[int, int, int]]] = {
    "rel-night-2001": ((20, 30, 120), (230, 200, 40)),
    "rel-night-deluxe": ((120, 20, 90), (240, 240, 240)),
    "rel-road-single": ((30, 120, 60), (250, 250, 210)),
    "rel-road-album": ((200, 90, 30), (20, 20, 20)),
}
FULL_PX = 600
THUMB_PX = 250


def picture(mbid: str, thumb: bool = True) -> Optional[bytes]:
    """The PNG the fake archive serves for a release's front cover, or None."""
    colours = COVERS.get(mbid)
    if colours is None:
        return None
    n = THUMB_PX if thumb else FULL_PX
    a = np.zeros((n, n, 3), dtype=np.uint8)
    a[:, : n // 2] = colours[0]
    a[:, n // 2:] = colours[1]
    a[: n // 8, :, :] = 255 - a[: n // 8, :, :]  # a band at the top, so up and down differ too
    return to_png(a)


class FakeCoverArt:
    """A threaded HTTP server on 127.0.0.1 answering like the Cover Art Archive."""

    def __init__(self, delay: float = 0.0):
        self.requests: List[str] = []
        self.agents: List[str] = []
        self.fail_next: List[int] = []   # statuses to answer the next /release/ requests with
        self.delay = delay               # seconds each picture takes (to see that downloads do not overlap)
        self.active = 0
        self.max_active = 0
        self.big: Dict[str, int] = {}    # release id -> bytes of padding after its full-size picture
        self._lock = threading.Lock()
        fake = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *a):  # quiet
                pass

            def _send(self, status: int, body: bytes = b"", ctype: str = "text/plain", location: str = "") -> None:
                with fake._lock:  # (answered: the client may ask for the next one as soon as it has read this)
                    fake.active -= 1
                self.send_response(status)
                self.send_header("Content-Type", ctype)
                self.send_header("Content-Length", str(len(body)))
                if location:
                    self.send_header("Location", location)
                if status == 503:
                    self.send_header("Retry-After", "1")
                self.end_headers()
                self.wfile.write(body)

            def do_GET(self):
                with fake._lock:
                    fake.requests.append(self.path)
                    fake.agents.append(self.headers.get("User-Agent", ""))
                    fake.active += 1
                    fake.max_active = max(fake.max_active, fake.active)
                self._answer()

            def _answer(self):
                parts = self.path.strip("/").split("/")
                if len(parts) == 3 and parts[0] == "release" and parts[2] in ("front", "front-250"):
                    if fake.fail_next:
                        return self._send(fake.fail_next.pop(0), b"busy")
                    if parts[1] not in COVERS:
                        return self._send(404, b"No cover art found for release " + parts[1].encode())
                    name = parts[1] + ("-250" if parts[2] == "front-250" else "")
                    return self._send(307, b"See: /img/" + name.encode(), location=f"/img/{name}.png")
                if len(parts) == 2 and parts[0] == "img" and parts[1].endswith(".png"):
                    name = parts[1][:-4]
                    thumb = name.endswith("-250")
                    mbid = name[:-4] if thumb else name
                    data = picture(mbid, thumb)
                    if data is None:
                        return self._send(404, b"Not Found")
                    if fake.delay:
                        time.sleep(fake.delay)
                    if not thumb:
                        data += b"\0" * fake.big.get(mbid, 0)
                    return self._send(200, data, "image/png")
                return self._send(404, b"Not Found")

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}"
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def asked(self, kind: str = "release") -> List[str]:
        """The requests whose path starts with /<kind>/ (the redirects' targets are /img/)."""
        return [p for p in self.requests if p.startswith(f"/{kind}/")]

    def close(self) -> None:
        self.server.shutdown()
        self.server.server_close()
