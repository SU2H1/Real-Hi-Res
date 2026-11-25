#!/usr/bin/env python3
"""Write rust/crates/core/tests/python_jacket.json: the Python answers of the 2.5 jacket
work for the Rust port to match (jacket_vectors.rs): realhires/jacket.py (the turn's
timing and geometry, the measures of the back), covers.back_picture() (the cover
mirrored and blurred; the pixels are equal, since both versions use whole numbers) and
realhires/coverart.py (the Cover Art Archive's addresses, what MusicBrainz is asked,
the texts of every outcome of a download).

The test pictures are made from a formula the Rust test repeats, the MusicBrainz
answers come from tests/fake_musicbrainz.py; nothing is downloaded.  Run it after
changing one of those modules (tests/test_jacket_25.py checks that the file is current;
it passes another output path).
"""

import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, ROOT)
sys.path.insert(0, os.path.join(ROOT, "tests"))

import numpy as np  # noqa: E402

import fake_musicbrainz  # noqa: E402
from realhires import coverart, covers, jacket, musicbrainz  # noqa: E402

OUT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, "rust", "crates", "core", "tests", "python_jacket.json")


def picture(w: int, h: int, seed: int) -> np.ndarray:
    """A picture made from numbers (the Rust test makes the same): (h, w, 3) uint8."""
    y, x = np.mgrid[0:h, 0:w].astype(np.int64)
    return np.stack([(x * 7 + y * 13 + c * 29 + ((x * y + seed) % 17) * 5 + seed * 3) % 256 for c in range(3)],
                    axis=2).astype(np.uint8)


# ---- jacket.py
steps = [0.0, 0.05, 0.1, 0.25, 0.4, 0.49, 0.5, 0.51, 0.6, 0.75, 0.9, 0.999, 1.0, -0.2, 1.3]
frames = []
for p in steps:
    for d in (1, -1):
        f = jacket.turn_frame(p, d)
        frames.append({"p": p, "dir": d, "other": f.other, "width": f.width, "far_right": f.far_right, "far": f.far,
                       "dark": f.dark})
drags = [{"dx": dx, "width": w, "progress": jacket.drag_progress(dx, w), "finishes": jacket.drag_finishes(dx, w)}
         for w in (480.0, 320.0, 200.0, 160.0, 120.0, 0.0)
         for dx in (0.0, 5.0, -29.9, 30.0, -30.1, 40.0, 50.1, 59.0, 60.0, -60.5, 80.0, 119.0, 121.0, -480.0, 900.0)]
rests = [{"p": p, "finish": fin, "whole": whole, "ms": jacket.rest_ms(p, fin, whole)}
         for p in (0.0, 0.1, 0.25, 0.5, 0.77, 1.0, 1.4, -1.0) for fin in (True, False) for whole in (360, 0, 1000)]
moves = [{"start": a, "target": b, "elapsed": e, "total": t, "p": jacket.progress_at(a, b, e, t)}
         for a, b in ((0.0, 1.0), (0.3, 1.0), (0.3, 0.0), (0.8, 0.0)) for e, t in ((0, 360), (36, 360), (90, 360),
                                                                                  (180, 360), (300, 360), (360, 360),
                                                                                  (400, 360), (-5, 360), (10, 0))]
layouts = []
for size in (1, 100, 120, 160, 200, 219, 220, 240, 280, 299, 300, 320, 360, 399, 400, 440, 480, 600):
    L = jacket.back_layout(size)
    layouts.append({k: getattr(L, k) for k in L.__dataclass_fields__})
scrolls = []
for total, rows in ((0, 5), (4, 5), (5, 5), (6, 5), (20, 11), (100, 4), (13, 12)):
    for first in (-3, 0, 1, 7, 99):
        for current in (-1, 0, 3, 12):
            for length in (0, 330):
                scrolls.append({"first": first, "total": total, "rows": rows, "current": current, "length": length,
                                "clamp": jacket.clamp_first(first, total, rows),
                                "first_for": jacket.first_for(current, first, total, rows),
                                "bar": list(jacket.scroll_bar(first, total, rows, length))})
mixes = [{"a": list(a), "b": list(b), "amount": k, "out": list(jacket.mix(a, b, k))}
         for a, b in (((0, 0, 0), (255, 255, 255)), ((63, 20, 200), (255, 255, 255)), ((250, 128, 3), (10, 20, 30)))
         for k in (0.0, 0.12, 0.5, 0.7, 0.84, 1.0, 1.5, -1.0)]
jacket_json = {
    "constants": {"FLIP_MS": jacket.FLIP_MS, "FADE_MS": jacket.FADE_MS, "DARKEN": jacket.DARKEN,
                  "PERSPECTIVE": jacket.PERSPECTIVE, "SWIPE_PX": jacket.SWIPE_PX, "SCROLL_PX": jacket.SCROLL_PX,
                  "HOVER_TINT": jacket.HOVER_TINT, "SECONDARY": jacket.SECONDARY, "SIDES": list(jacket.SIDES),
                  "NO_ANIMATION_ENV": jacket.NO_ANIMATION_ENV},
    "ease": [{"t": t, "out": jacket.ease(t)} for t in (-1.0, 0.0, 0.1, 0.25, 0.4, 0.5, 0.6, 0.75, 0.9, 1.0, 2.0)],
    "frames": frames, "drags": drags, "rests": rests, "moves": moves, "layouts": layouts, "scrolls": scrolls,
    "mixes": mixes,
    "other_side": {s: jacket.other_side(s) for s in ("cover", "songs", "")},
}

# ---- covers.back_picture()
backs = []
for w, h, seed, size, scrim in ((8, 8, 1, 16, 0.0), (100, 60, 2, 48, 0.5), (5, 3, 3, 10, 0.0), (130, 130, 4, 64, 0.0),
                                (200, 200, 5, 33, 0.5), (1, 1, 6, 4, 0.0), (40, 40, 7, 0, 0.35), (70, 90, 8, 20, 1.0)):
    out = covers.back_picture(picture(w, h, seed), size, scrim)
    backs.append({"w": w, "h": h, "seed": seed, "size": size, "scrim": scrim, "out_size": int(out.shape[0]),
                  "hex": out.tobytes().hex()})
dark = [{"amount": k, "hex": covers.darken(picture(6, 4, 9), k).tobytes().hex()} for k in (0.0, 0.2, 0.35, 0.5, 1.0, 2.0)]
covers_json = {
    "constants": {"BACK_SMALL": covers.BACK_SMALL, "BACK_BLUR_RADIUS": covers.BACK_BLUR_RADIUS,
                  "BACK_BLUR_PASSES": covers.BACK_BLUR_PASSES, "BACK_SCRIM": covers.BACK_SCRIM},
    "picture_check": picture(4, 3, 1).tobytes().hex(), "backs": backs, "darken": dark,
}

# ---- coverart.py
record = []
client = musicbrainz.MusicBrainzClient("https://musicbrainz.org", transport=fake_musicbrainz.transport(record),
                                       min_interval=0.0, clock=lambda: 0.0, sleep=lambda s: None)
found = []
for album, artist in (("Night Drive", "The Example Band"), ("Night Drive", ""), ("Lonely Road", "Solo Artist"),
                      ("Open Country", ""), ("Nothing Like It", "Nobody"), ("road", "")):
    record.clear()
    rels = coverart.find_releases(client, album, artist)
    found.append({"album": album, "artist": artist, "url": record[0],
                  "answer": json.dumps(fake_musicbrainz.answer(record[0])[1]),  # what the fake MusicBrainz says
                  "releases": [{"id": r.id, "title": r.title, "artist": r.artist, "facts": coverart.release_facts(r)}
                               for r in rels]})

PNG = covers.PNG_START + b"rest"
answers = [(200, "png"), (200, "jpeg"), (200, "text"), (200, "big"), (200, "empty"), (404, "text"), (429, "text"),
           (503, "text"), (500, "text"), (403, "text"), (301, "text"), ("network", "no route to host"),
           ("timeout", ""), ("invalid", "unknown url type: ftp")]
BODIES = {"png": PNG, "jpeg": b"\xff\xd8\xff\xe0rest", "text": b"<html>busy</html>", "big": PNG + b"\0" * 40,
          "empty": b""}
outcomes = []
for base in ("https://coverartarchive.org", "http://127.0.0.1:8123/"):
    for status, kind in answers:
        def download(url, headers, timeout, limit, status=status, kind=kind):
            if status == "network":
                raise OSError(kind)
            if status == "timeout":
                raise TimeoutError()
            if status == "invalid":
                raise ValueError(kind)
            return status, {}, BODIES[kind][:limit + 1]

        c = coverart.CoverArtClient(base, download=download, max_bytes=32)
        try:
            data = c.front("rel-1", True)
            out = {"result": "none" if data is None else "picture", "bytes": 0 if data is None else len(data)}
        except coverart.CoverArtBusy as exc:
            out = {"result": "busy", "message": str(exc)}
        except coverart.CoverArtError as exc:
            out = {"result": "error", "message": str(exc)}
        outcomes.append({"server": base, "status": status, "kind": kind, **out})
coverart_json = {
    "constants": {"DEFAULT_SERVER": coverart.DEFAULT_SERVER, "SERVER_ENV": coverart.SERVER_ENV,
                  "MAX_BYTES": coverart.MAX_BYTES, "MAX_RELEASES": coverart.MAX_RELEASES,
                  "THUMB_PX": coverart.THUMB_PX, "TIMEOUT_S": coverart.TIMEOUT_S, "CREDIT": coverart.CREDIT,
                  "ABOUT_CREDIT": coverart.ABOUT_CREDIT},
    "servers": [{"in": s, "out": coverart.normalize_server(s)}
                for s in ("", "  ", "coverartarchive.org", "https://coverartarchive.org/", "http://127.0.0.1:9/",
                          " example.org/caa// ")],
    "urls": [{"mbid": m, "thumb": t, "base": b, "url": coverart.front_url(m, t, b)}
             for m in ("76df3287-6cda-33eb-8e9a-044b5e15ffdd", "rel night/2001", "é?&#") for t in (True, False)
             for b in ("https://coverartarchive.org", "http://127.0.0.1:8123/")],
    "terms": [{"in": list(t), "out": list(coverart.search_terms(*t))}
              for t in (("Band", "Singer", "Album", "Song"), ("", "Singer", " Big   Album ", "Song"),
                        ("Band", "Singer", "", "Song"), ("Band", "", "", " A  Song "), ("", "", "", ""),
                        ("", "", "Album", ""))],
    "megabytes": [{"n": n, "out": coverart.megabytes(n)} for n in (0, 32, 1048576, 20 * 1024 * 1024, 52428799)],
    "found": found, "outcomes": outcomes,
}

with open(OUT, "w", encoding="utf-8") as f:
    json.dump({"jacket": jacket_json, "covers": covers_json, "coverart": coverart_json}, f, ensure_ascii=False,
              indent=1, sort_keys=True)
    f.write("\n")
print(f"wrote {OUT}")
