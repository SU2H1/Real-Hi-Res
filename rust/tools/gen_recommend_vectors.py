#!/usr/bin/env python3
"""Write rust/crates/core/tests/python_recommend.json: the Python answers of
realhires/recommend.py (2.5, For You) for the Rust port to match
(recommend_vectors.rs).

The libraries and listening histories are invented here with a small integer
generator, so the file is the same on every run: songs that are liked but were never
played, songs skipped early again and again, songs heard to the end long ago and
recently, disliked songs, songs whose files are gone, plays without a known duration,
songs with the very same history (ties), artists with many songs (the limits per
artist), a library of one artist, and the Daily Mix on several dates.  Run it after
changing realhires/recommend.py (tests/test_foryou_25.py checks that the file is
current; it passes another output path).
"""

import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, ROOT)

from realhires import recommend as R  # noqa: E402
from realhires.musiclib import Track  # noqa: E402
from realhires.stats import Play  # noqa: E402

OUT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, "rust", "crates", "core", "tests",
                                                         "python_recommend.json")
NOW = 1795600000.0  # 2026-11-25
DAY = 86400.0
DAYS = ["2026-11-25", "2026-11-26", "2026-12-31", "2027-01-01"]


class Lcg:
    """The generator of the invented histories (the C library's old rand())."""

    def __init__(self, seed: int):
        self.x = seed

    def next(self, n: int) -> int:
        self.x = (1103515245 * self.x + 12345) % (1 << 31)
        return (self.x >> 8) % n


def track(path, title="", artist="", album="", album_artist="", genre="", duration=240.0) -> Track:
    return Track(path, title=title, artist=artist, album=album, album_artist=album_artist, genre=genre,
                 duration=duration)


def play(path, days_ago, seconds, duration, now=NOW) -> Play:
    return Play(path, now - days_ago * DAY, float(seconds), float(duration))


def play_json(p: Play) -> dict:
    return {"path": p.path, "started": p.started, "seconds": p.seconds, "duration": p.duration}


def track_json(t: Track) -> dict:
    return {"path": t.path, "title": t.title, "artist": t.artist, "album": t.album, "album_artist": t.album_artist,
            "genre": t.genre, "duration": t.duration}


LIBRARIES: dict = {}   # the invented libraries and histories, each written once
HISTORIES: dict = {}


def scenario(name, library, history, liked=(), disliked=(), now=NOW, days=DAYS) -> dict:
    tracks, plays = LIBRARIES[library], HISTORIES[history]
    liked, disliked = sorted(liked), sorted(disliked)
    stats = R.song_stats(plays, liked, disliked, now)
    return {
        "name": name, "now": now, "library": library, "history": history, "liked": liked, "disliked": disliked,
        "stats": {p: s.to_json() for p, s in sorted(stats.items())},
        "candidates": R.candidates(tracks, plays, liked, disliked, now),
        "days": {d: R.mixes(tracks, plays, liked, disliked, now, d) for d in days},
    }


def history(plays) -> list:
    return sorted(plays, key=lambda p: (p.started, p.path, p.seconds))  # oldest first, as the library gives them


# ------------------------------------------------------------- the big library ---
ARTISTS = [  # name, genre, songs
    ("Aurora Lane", "Pop", 14), ("aurora lane feat. The Others", "Pop", 3), ("Brass Monkeys", "Jazz", 9),
    ("Die Straße", "Rock", 8), ("DIE STRASSE", "Rock", 4), ("東京スカイ", "J-Pop", 10), ("Émile Zola Trio", "Jazz", 7),
    ("Fjord; Glacier", "Ambient", 6), ("Hollow Pines", "Rock", 12), ("Ivory & Co", "Pop", 5),
    ("", "", 6), ("Kite", "", 5), ("The Long Tail", "Rock", 40),
]


def big_library():
    tracks = []
    for a, (name, genre, n) in enumerate(ARTISTS):
        for s in range(n):
            album = f"Album {a}-{s // 5}" if name else ""
            album_artist = "Various" if a == 9 else ("Compilation Crew" if a == 10 and s < 3 else "")
            duration = 0.0 if (a == 11 and s == 4) else 150.0 + 13.0 * ((a * 7 + s * 3) % 17)
            tracks.append(track(f"/music/{a:02d}/{s:02d} song.flac", f"Song {a}-{s}", name, album, album_artist,
                                genre, duration))
    return tracks


def big_history(tracks):
    rng = Lcg(2025)
    by = {t.path: t for t in tracks}
    plays, liked, disliked = [], set(), set()
    for i, t in enumerate(tracks):
        kind = rng.next(12)
        d = t.duration or 200.0
        if kind == 0:      # never played
            continue
        if kind == 1:      # liked but never played
            liked.add(t.path)
            continue
        if kind == 2:      # skipped early again and again
            for k in range(3 + rng.next(30)):
                plays.append(play(t.path, 1 + rng.next(120) + k / 7.0, 3 + rng.next(12), t.duration))
        elif kind == 3:    # heard to the end long ago
            for k in range(3 + rng.next(25)):
                plays.append(play(t.path, 200 + rng.next(400) + k / 5.0, d, t.duration))
        elif kind == 4:    # heard to the end recently
            for k in range(1 + rng.next(25)):
                plays.append(play(t.path, rng.next(28) + k / 9.0, d, t.duration))
        elif kind == 5:    # long ago and recently
            for k in range(2 + rng.next(10)):
                plays.append(play(t.path, 300 + rng.next(100) + k / 3.0, d, t.duration))
            for k in range(1 + rng.next(6)):
                plays.append(play(t.path, rng.next(20) + k / 3.0, d * (50 + rng.next(51)) / 100.0, t.duration))
        elif kind == 6:    # disliked, with or without plays
            disliked.add(t.path)
            for k in range(rng.next(4)):
                plays.append(play(t.path, 5 + rng.next(60), 5 + rng.next(40), t.duration))
        elif kind == 7:    # liked and heard, some months ago
            liked.add(t.path)
            for k in range(1 + rng.next(8)):
                plays.append(play(t.path, 61 + rng.next(200) + k / 4.0, d * (70 + rng.next(31)) / 100.0, t.duration))
        elif kind == 8:    # plays logged without a duration (the file was unreadable then)
            for k in range(1 + rng.next(6)):
                plays.append(play(t.path, rng.next(90) + k / 2.0, 10 + rng.next(200), 0.0))
        elif kind == 9:    # a bit of everything
            for k in range(1 + rng.next(20)):
                plays.append(play(t.path, rng.next(500) + k / 11.0, d * (1 + rng.next(100)) / 100.0, t.duration))
        elif kind == 10:   # half heard, a few times
            for k in range(2 + rng.next(5)):
                plays.append(play(t.path, 10 + rng.next(80), d / 2.0, t.duration))
        else:              # once, yesterday, and liked on every third
            plays.append(play(t.path, 1.0, d, t.duration))
            if i % 3 == 0:
                liked.add(t.path)
    # songs whose files are gone: played and liked, but not in the library
    for k in range(12):
        plays.append(play("/music/gone/old favourite.flac", 3 + k, 240.0, 240.0))
    liked.add("/music/gone/old favourite.flac")
    liked.add("/music/gone/never there.flac")
    disliked.add("/music/gone/disliked.flac")
    plays.append(play("/music/00/00 song.flac", -2.0, 200.0, by["/music/00/00 song.flac"].duration))  # the clock was wrong
    return plays, liked, disliked


big = big_library()
big_plays, big_liked, big_disliked = big_history(big)

# --------------------------------------------------------------------- ties ---
# songs with the very same history: only the path decides, in every mix
ties = [track(f"/t/{name}.flac", name, artist, f"LP {artist}", "", "Folk", 180.0)
        for name, artist in (("zeta", "One"), ("alpha", "Two"), ("Beta", "Three"), ("gamma", "Four"), ("delta", "Five"),
                             ("épsilon", "Six"), ("eta", "Seven"), ("theta", "Eight"), ("Iota", "One"),
                             ("kappa", "Two"), ("lambda", "Nine"), ("mu", "Ten"), ("nu", "Eleven"), ("xi", "Twelve"))]
ties_plays = []
for t in ties[:8]:          # heard to the end three times this week, at the same moments
    for k in range(3):
        ties_plays.append(play(t.path, 1 + k, 180.0, 180.0))
for t in ties[8:11]:        # and three times a year ago
    for k in range(3):
        ties_plays.append(play(t.path, 400 + k, 180.0, 180.0))
ties_liked = [ties[11].path, ties[12].path]

# ------------------------------------------------------------- one artist only ---
solo = [track(f"/solo/{i:02d}.flac", f"Solo {i}", "Only One" if i % 2 else "ONLY ONE feat. A Guest", "The Album", "",
              "Blues", 200.0) for i in range(30)]
solo_plays = [play(t.path, 1 + (i * 5) % 23 + k, 200.0 if i % 4 else 12.0, 200.0)
              for i, t in enumerate(solo[:22]) for k in range(1 + i % 4)]
solo_plays += [play(t.path, 100 + i + k, 200.0, 200.0) for i, t in enumerate(solo[:6]) for k in range(4)]

# ------------------------------------------------------------- the small cases ---
few = [track(f"/few/{i}.flac", f"Few {i}", f"Artist {i}", "", "", "", 100.0) for i in range(4)]
few_plays = [play(t.path, 1 + i, 100.0, 100.0) for i, t in enumerate(few) for _ in range(2)]
untagged = [track(f"/raw/{i:02d}.wav", duration=120.0) for i in range(20)]
untagged_plays = [play(t.path, 2 + i, 120.0, 120.0) for i, t in enumerate(untagged[:16]) for _ in range(1 + i % 3)]

LIBRARIES.update({"big": big, "ties": ties, "solo": solo, "few": few, "untagged": untagged, "none": [],
                  "ties twice": ties + ties[:3]})
HISTORIES.update({"big": history(big_plays), "ties": history(ties_plays), "solo": history(solo_plays),
                  "few": history(few_plays), "untagged": history(untagged_plays), "none": []})
scenarios = [
    scenario("big", "big", "big", big_liked, big_disliked),
    scenario("big a year later", "big", "big", big_liked, big_disliked, now=NOW + 365 * DAY, days=["2027-11-25"]),
    scenario("ties", "ties", "ties", ties_liked),
    scenario("one artist", "solo", "solo", [solo[25].path], [solo[0].path]),
    scenario("few songs", "few", "few"),
    scenario("untagged", "untagged", "untagged"),
    scenario("likes only", "big", "none", [t.path for t in big[::9]], [big[1].path]),
    scenario("nothing", "big", "none"),
    scenario("no library", "none", "big", big_liked, big_disliked, days=DAYS[:1]),
    scenario("the same song twice", "ties twice", "ties", ties_liked, days=DAYS[:1]),
]

# ------------------------------------------------------------------- the parts ---
names = {"a1": "a", "a2": "a", "a3": "a", "a4": "a", "b1": "b", "b2": "b", "c1": "c", "c2": "c", "c3": "c",
         "u1": "", "u2": "", "d1": "d"}
orders = [["a1", "a2", "a3", "b1", "b2"], ["a1", "b1", "c1", "c2", "c3"], ["a1", "a2", "a3", "a4"],
          ["a1", "a2", "a3", "b1"], ["a1", "a2", "u1", "u2", "a3", "a4"], ["b1", "a1", "a2", "c1", "a3", "c2", "a4"],
          ["u1", "u2"], [], ["a1"], ["a1", "b1", "a2", "b2", "c1", "c2", "c3", "d1", "a3"],
          ["c1", "c2", "c3", "a1", "a2", "a3", "a4", "b1", "b2", "d1", "u1"]]
spread = [{"paths": o, "out": R.spread_artists(o, names.get)} for o in orders]
many = [f"{'abcde'[i % 5]}{i:02d}" for i in range(40)] + [f"a{i:02d}" for i in range(40, 60)]
many_names = {p: p[0] for p in many}
limit = [{"paths": o, "artists": n, "size": size, "per_artist": per,
          "out": R.limit_artists(o, n.get, size, per)}
         for o, n, size, per in ((orders[10], names, 50, 4), (orders[10], names, 5, 1), (orders[2], names, 50, 2),
                                 (many, many_names, 50, 4), (many, many_names, 30, 4), (many, many_names, 10, 4),
                                 ([f"a{i:02d}" for i in range(40, 60)], many_names, 50, 4),
                                 ([f"a{i:02d}" for i in range(40, 60)], many_names, 8, 4), ([], {}, 50, 4),
                                 (many, {p: "" for p in many}, 50, 4))]
day_paths = [t.path for t in big[:12]] + ["/m/日本語.flac", "/m/a b.mp3", ""]
key_tracks = [track("/k/1", artist="A; B"), track("/k/2", artist="Earth, Wind & Fire"), track("/k/3", artist=" Name  "),
              track("/k/4", artist="", album_artist="Various Artists"), track("/k/5"),
              track("/k/6", artist="X feat. Y", album="  The ALBUM ", album_artist="Z", genre=" Hip-Hop "),
              track("/k/7", artist="STRAẞE (ft. İstanbul)", album="Größe"), track("/k/8", artist="feat. Nobody"),
              track("/k/9", artist="；", album="", album_artist="AA")]

data = {
    "constants": {k: getattr(R, k) for k in (
        "HALF_LIFE_DAYS", "PRIOR_R", "PRIOR_N", "NO_DURATION_SECONDS", "LIKE_POINTS", "RETENTION_POINTS",
        "TIME_UNIT", "MIN_SONGS", "MAX_PER_ARTIST", "DAILY_PER_ARTIST", "MIN_FILL", "FAVOURITES_SIZE",
        "ON_REPEAT_SIZE", "ON_REPEAT_DAYS", "REDISCOVER_SIZE", "REDISCOVER_DAYS", "REDISCOVER_PLAYS",
        "REDISCOVER_RETENTION", "DISCOVER_SIZE", "DAILY_SIZE", "DAILY_REDISCOVER", "DAILY_DISCOVER", "DAILY_POOL")},
    "mixes": [list(m) for m in R.MIXES], "how_note": R.HOW_NOTE, "empty_note": R.EMPTY_NOTE,
    "libraries": {k: [track_json(t) for t in v] for k, v in LIBRARIES.items()},
    "histories": {k: [play_json(p) for p in v] for k, v in HISTORIES.items()},
    "scenarios": scenarios,
    "spread": spread, "spread_artists": names, "limit": limit,
    "keys": [[x, R.key(x)] for x in (0.0, 1e-7, 4.9e-7, 5.1e-7, -4.9e-7, -5.1e-7, 1.0, -2.5, 6.123456789, 123456.75)],
    "heard": [[s, d, R.heard(s, d)] for s, d in ((10, 240), (240, 240), (300, 240), (0, 240), (-5, 240), (10, 0),
                                                 (30, 0), (45, 0), (10, -3), (0.5, 1e-9))],
    "weights": [[a, R.weight(a)] for a in (0.0, -100.0, DAY, 60 * DAY, 120 * DAY, 365.25 * DAY, 12345.678)],
    "scores": [[liked, disliked, r, s, R.score_of(liked, disliked, r, s)]
               for liked, disliked, r, s in ((False, False, 0.5, 0.0), (True, False, 0.5, 0.0),
                                             (False, True, 0.5, 0.0), (False, False, 0.9, 180.0),
                                             (True, False, 0.1, 1260.0), (False, False, 0.05, 3.0))],
    "day_order": [{"day": d, "out": R.day_order(d, day_paths), "hashes": [R.day_hash(d, p) for p in day_paths]}
                  for d in DAYS],
    "day_paths": day_paths,
    "track_keys": [[track_json(t), R.artist_key(t), R.album_key(t), R.genre_key(t)] for t in key_tracks],
}
with open(OUT, "w", encoding="utf-8", newline="\n") as fh:
    json.dump(data, fh, indent=0, ensure_ascii=False)
    fh.write("\n")
shown = {s["name"]: {d: [(m["id"], len(m["paths"])) for m in ms] for d, ms in list(s["days"].items())[:1]}
         for s in scenarios}
for name, v in shown.items():
    print(name, v)
print(len(scenarios), "scenarios ->", OUT)
