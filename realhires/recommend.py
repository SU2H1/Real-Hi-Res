"""For You: playlists made from likes, retention and listening time (2.5).

"Make playlist algorithm based on user like and retention for the songs and time
played back."  Everything is worked out on this computer from three things the player
already keeps: the songs the user liked and disliked (`playerstate`), and the play
history (`musiclib.Library.plays()`: when each song was played, for how many seconds,
and how long the song is).  Nothing is random: the same library, history and date give
the same mixes in the Python and in the Rust version
(`rust/crates/core/src/recommend.rs`, checked against
`rust/crates/core/tests/python_recommend.json`, which
`rust/tools/gen_recommend_vectors.py` writes).

Per song (`song_stats()`)
-------------------------
For each play of a song:

* `r`, the part of the song heard: `min(1, seconds / duration)`; a play without a
  known duration: `min(1, seconds / NO_DURATION_SECONDS)` (30).
* `w`, its recency weight: `0.5 ** (age in days / HALF_LIFE_DAYS)` (60): a play of two
  months ago counts half, one of a year ago about a seventieth.

**Retention** is the smoothed, recency-weighted mean of `r`:
`(Σ w·r + PRIOR_R·PRIOR_N) / (Σ w + PRIOR_N)` with `PRIOR_R` 0.5 and `PRIOR_N` 2, so
one play never decides it and old plays fade back to "not known" (0.5).

**Time** is `Σ w·seconds·r`: the seconds of a play count by the part of the song that
was heard.  (The first formula counted every second, `Σ w·seconds`; then a song
skipped after ten seconds a hundred times collected a quarter of an hour of
"listening" and rose into the favourites.  With `·r` a skipped song's seconds count
for almost nothing and a song heard to the end counts in full.)

**Score** = `LIKE_POINTS` (2.0) when liked, minus `LIKE_POINTS` when disliked,
+ `RETENTION_POINTS` (4.0) × (retention − 0.5) + log2(1 + time / `TIME_UNIT`) (180:
three minutes of recent listening to the end are worth one point, seven minutes two,
fifteen three).  A song never played and neither liked nor disliked scores exactly 0.

The *plain* retention and score are the same without the recency weight (`w` = 1):
what the user thought of a song whenever they played it (Rediscover uses them).

The mixes (`mixes()`)
---------------------
Only songs of the library that are not disliked are ever in a mix.  In this order; a
mix with fewer than `MIN_SONGS` (5) songs is left out:

* **Favourites Mix** (`favourites`): the songs with the highest scores above 0, at
  most 50.
* **On Repeat** (`on_repeat`): the songs with a score above 0 that were heard most in
  the last 30 days, by counted plays (`stats.counted()`), then seconds; at most 30.
* **Rediscover** (`rediscover`): songs that were played, but not in the last 60 days,
  and that are liked or were heard to the end often (at least 3 counted plays and a
  plain retention of 0.6 or more), by the plain score (above 0); at most 30.
* **New For You** (`discover`): songs without a counted play that are not liked,
  ranked by how much the user likes their artist, album and genre: the mean score of
  the songs the user knows (a counted play, a like or a dislike) that share it, the
  artist counting twice, `(2·artist + album + genre) / 4`, plus the song's own score
  (below 0 when it was skipped early); only those above 0, at most 30.
* **Daily Mix** (`daily`): 40 songs for the day: up to 8 of Rediscover's candidates,
  8 of New For You's and the rest (24 and what the other two could not give) of the
  favourites'.  Each part is drawn from the best three times as many candidates (24,
  24, 72) in the order of SHA-1(`YYYY-MM-DD` + path), which is also the mix's order,
  so it changes every day and both apps show the same one.

In every mix no artist has more than `MAX_PER_ARTIST` (4) songs (Daily Mix:
`DAILY_PER_ARTIST`, 3).  The artist of a song is the first name of its artist field
(`musiclib.artist_pieces()`, else of its album artist) in any case; songs without one
are not limited.  A library of one or two artists would get no mix at all that way, so
when a mix stays under `MIN_FILL` (12) songs, the limit rises by one song at a time
until it has 12 or no candidate is left.  Then two songs of one artist do not follow
each other where the mix can avoid it (`spread_artists()`), keeping the order
otherwise.

The same answers in both languages
----------------------------------
Sums run over the plays in the order given (oldest first, as `Library.plays()` gives
them) and over the songs sorted by path, and every tie is broken by the path.  `r`,
the plain retention and the seconds are sums, products and quotients of the same
doubles in the same order, so they are equal to the last bit.  `0.5 ** x` and `log2`
come from the platform's C library and may differ in the last bit between platforms,
so nothing is ordered or compared on them directly: scores, ranks and seconds are
compared as whole millionths (`key()`: `floor(x · 1e6 + 0.5)`), where a difference in
the sixteenth digit cannot change an order (short of a value that lies within 1e-10 of
a half millionth).
"""

from __future__ import annotations

import hashlib
import math
import time
from dataclasses import dataclass
from typing import Callable, Dict, Iterable, List, Optional, Sequence

from .musiclib import Track, artist_pieces
from .stats import Play, counted

HALF_LIFE_DAYS = 60.0        # a play's weight halves every 60 days
PRIOR_R = 0.5                # the retention of a song nothing is known about
PRIOR_N = 2.0                # ... worth two plays
NO_DURATION_SECONDS = 30.0   # a play of a song without a known duration is whole after this
LIKE_POINTS = 2.0
RETENTION_POINTS = 4.0
TIME_UNIT = 180.0            # seconds of recent listening to the end that are worth one point

MIN_SONGS = 5                # a mix with fewer songs is left out
MAX_PER_ARTIST = 4
DAILY_PER_ARTIST = 3
MIN_FILL = 12                # the artist limit gives way until a mix has this many songs
FAVOURITES_SIZE = 50
ON_REPEAT_SIZE = 30
ON_REPEAT_DAYS = 30.0
REDISCOVER_SIZE = 30
REDISCOVER_DAYS = 60.0
REDISCOVER_PLAYS = 3
REDISCOVER_RETENTION = 0.6
DISCOVER_SIZE = 30
DAILY_SIZE = 40
DAILY_REDISCOVER = 8
DAILY_DISCOVER = 8
DAILY_POOL = 3               # each part of the Daily Mix is drawn from this many times its songs
DAY = 86400.0

# id, name, description (English; the apps show them through tr())
MIXES = (
    ("favourites", "Favourites Mix", "The songs you like and listen to the most."),
    ("on_repeat", "On Repeat", "What you played most in the last 30 days."),
    ("rediscover", "Rediscover", "Songs you loved and have not played for a while."),
    ("discover", "New For You", "Songs you have not played yet, close to what you like."),
    ("daily", "Daily Mix", "A different mix every day: favourites, old loves and new songs."),
)
MIX_IDS = tuple(m[0] for m in MIXES)
HOW_NOTE = ("Made on this computer from your likes, how much of each song you listen to and the time you spend "
            "with it.")
EMPTY_NOTE = "Play and like some songs, and your mixes will appear here."


@dataclass
class SongStats:
    """What the history says about one song."""
    path: str
    plays: int = 0               # counted plays (stats.counted())
    skips: int = 0               # logged plays that are not counted
    seconds: float = 0.0         # every logged second
    last_played: float = 0.0     # Unix time of the newest play, 0.0 = never
    retention: float = PRIOR_R   # recency-weighted, smoothed part of the song heard
    time: float = 0.0            # recency-weighted seconds, each play's by the part heard
    score: float = 0.0
    plain_retention: float = PRIOR_R   # the same without the recency weight
    plain_score: float = 0.0
    recent_plays: int = 0        # counted plays of the last ON_REPEAT_DAYS days
    recent_seconds: float = 0.0  # seconds of the last ON_REPEAT_DAYS days
    liked: bool = False
    disliked: bool = False

    def to_json(self) -> dict:
        return dict(self.__dict__)


def key(x: float) -> int:
    """`x` in whole millionths: what scores and seconds are ordered and compared by."""
    return int(math.floor(x * 1e6 + 0.5))


def heard(seconds: float, duration: float) -> float:
    """The part of a song that one play heard, 0..1."""
    seconds = max(0.0, seconds)
    return min(1.0, seconds / duration) if duration > 0.0 else min(1.0, seconds / NO_DURATION_SECONDS)


def weight(age_seconds: float) -> float:
    """The recency weight of a play that old (a play "in the future" counts as now)."""
    return 0.5 ** (max(0.0, age_seconds) / DAY / HALF_LIFE_DAYS)


def score_of(liked: bool, disliked: bool, retention: float, seconds: float) -> float:
    like = (LIKE_POINTS if liked else 0.0) - (LIKE_POINTS if disliked else 0.0)
    return like + RETENTION_POINTS * (retention - PRIOR_R) + math.log2(1.0 + seconds / TIME_UNIT)


def song_stats(plays: Iterable[Play], liked: Iterable[str] = (), disliked: Iterable[str] = (),
               now: Optional[float] = None) -> Dict[str, SongStats]:
    """The statistics of every song that was played, liked or disliked, by its path
    (plays oldest first, as `Library.plays()` gives them)."""
    now = time.time() if now is None else float(now)
    liked, disliked = set(liked), set(disliked)
    out: Dict[str, SongStats] = {}
    sums: Dict[str, List[float]] = {}  # Σ w·r, Σ w, Σ w·seconds·r, Σ r, n, Σ seconds·r
    recent = now - ON_REPEAT_DAYS * DAY
    for p in plays:
        s = out.get(p.path)
        if s is None:
            s = out[p.path] = SongStats(p.path)
            sums[p.path] = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
        seconds = max(0.0, float(p.seconds))
        c = counted(p)
        if c:
            s.plays += 1
        else:
            s.skips += 1
        s.seconds += seconds
        s.last_played = max(s.last_played, float(p.started))
        if p.started >= recent:
            s.recent_seconds += seconds
            if c:
                s.recent_plays += 1
        r = heard(seconds, float(p.duration))
        w = weight(now - p.started)
        a = sums[p.path]
        a[0] += w * r
        a[1] += w
        a[2] += w * seconds * r
        a[3] += r
        a[4] += 1.0
        a[5] += seconds * r
    for path in sorted(liked | disliked):
        if path not in out:
            out[path] = SongStats(path)
            sums[path] = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
    for path, s in out.items():
        a = sums[path]
        s.liked, s.disliked = path in liked, path in disliked
        s.retention = (a[0] + PRIOR_R * PRIOR_N) / (a[1] + PRIOR_N)
        s.time = a[2]
        s.score = score_of(s.liked, s.disliked, s.retention, s.time)
        s.plain_retention = (a[3] + PRIOR_R * PRIOR_N) / (a[4] + PRIOR_N)
        s.plain_score = score_of(s.liked, s.disliked, s.plain_retention, a[5])
    return out


# --------------------------------------------------------------- artists ---
def artist_key(t: Track) -> str:
    """The artist a song counts for in the mixes' limits: the first name of its artist
    field (else of its album artist), in any case; "" without one."""
    pieces = artist_pieces(t.artist) or artist_pieces(t.album_artist)
    return pieces[0].casefold() if pieces else ""


def album_key(t: Track) -> str:
    """The album a song belongs to for New For You ("" without an album tag)."""
    album = t.album.strip().casefold()
    return album + "\x00" + (t.album_artist or t.artist).strip().casefold() if album else ""


def genre_key(t: Track) -> str:
    return t.genre.strip().casefold()


def limit_artists(paths: Sequence[str], artist_of: Callable[[str], str], size: int, per_artist: int) -> List[str]:
    """The first `size` of `paths` with at most `per_artist` songs of an artist (songs
    without an artist, "", are not limited), in their order.  While that leaves fewer
    than MIN_FILL songs and some were refused, the limit rises by one."""
    counts: Dict[str, int] = {}
    taken: List[tuple] = []
    rest = list(enumerate(paths))
    limit, target = per_artist, size
    while rest and len(taken) < target:
        left = []
        for i, p in rest:
            k = artist_of(p)
            if len(taken) < target and (not k or counts.get(k, 0) < limit):
                taken.append((i, p))
                if k:
                    counts[k] = counts.get(k, 0) + 1
            else:
                left.append((i, p))
        rest = left
        limit += 1
        target = min(size, MIN_FILL)
    return [p for _, p in sorted(taken)]


def spread_artists(paths: Sequence[str], artist_of: Callable[[str], str]) -> List[str]:
    """`paths` with two songs of one artist not following each other where that can be
    avoided, otherwise in their order: each place gets the first song left whose artist
    is not the one before it; an artist who has half of the songs left (or more) goes
    first, or their songs would meet at the end."""
    left = list(paths)
    keys = {p: artist_of(p) for p in left}
    counts: Dict[str, int] = {}
    for p in left:
        if keys[p]:
            counts[keys[p]] = counts.get(keys[p], 0) + 1
    out: List[str] = []
    prev = ""
    while left:
        n = len(left)
        pick = None
        for p in left:  # an artist who needs every other place from here on
            k = keys[p]
            if k and k != prev and 2 * counts[k] >= n:
                pick = p
                break
        if pick is None:
            pick = next((p for p in left if not keys[p] or keys[p] != prev), left[0])
        left.remove(pick)
        prev = keys[pick]
        if prev:
            counts[prev] -= 1
        out.append(pick)
    return out


# ------------------------------------------------------------------ the day ---
def day_of(now: float, localtime: Callable = time.localtime) -> str:
    """The local date of `now` as the Daily Mix uses it: "2026-11-25"."""
    t = localtime(now)
    return f"{t.tm_year:04d}-{t.tm_mon:02d}-{t.tm_mday:02d}"


def day_hash(day: str, path: str) -> str:
    return hashlib.sha1((day + path).encode("utf-8", "surrogateescape")).hexdigest()


def day_order(day: str, paths: Iterable[str]) -> List[str]:
    """`paths` in the day's order: by SHA-1(day + path), then the path."""
    return sorted(paths, key=lambda p: (day_hash(day, p), p))


# ---------------------------------------------------------------- the mixes ---
def candidates(tracks: Iterable[Track], plays: Iterable[Play], liked: Iterable[str] = (),
               disliked: Iterable[str] = (), now: Optional[float] = None) -> Dict[str, List[str]]:
    """The songs each mix chooses from, best first, before the limit per artist:
    {"favourites", "on_repeat", "rediscover", "discover"}."""
    now = time.time() if now is None else float(now)
    disliked = set(disliked)
    stats = song_stats(plays, liked, disliked, now)
    library = sorted({t.path: t for t in tracks}.values(), key=lambda t: t.path)  # each path once, by path
    songs = [t for t in library if t.path not in disliked]
    blank = SongStats("")

    def st(t: Track) -> SongStats:
        return stats.get(t.path, blank)

    favourites = sorted((t.path for t in songs if key(st(t).score) > 0), key=lambda p: (-key(stats[p].score), p))
    on_repeat = sorted((t.path for t in songs if st(t).recent_plays > 0 and key(st(t).score) > 0),
                       key=lambda p: (-stats[p].recent_plays, -key(stats[p].recent_seconds), p))
    before = now - REDISCOVER_DAYS * DAY
    rediscover = sorted(
        (t.path for t in songs
         if 0.0 < st(t).last_played < before and key(st(t).plain_score) > 0
         and (st(t).liked or (st(t).plays >= REDISCOVER_PLAYS and st(t).plain_retention >= REDISCOVER_RETENTION))),
        key=lambda p: (-key(stats[p].plain_score), p))
    # New For You: the mean score of the songs the user knows, per artist, album and genre
    means: List[Dict[str, List[float]]] = [{}, {}, {}]
    keys_of = (artist_key, album_key, genre_key)
    for t in library:
        s = stats.get(t.path)
        if s is None or not (s.plays or s.liked or s.disliked):
            continue
        for table, key_of in zip(means, keys_of):
            k = key_of(t)
            if k:
                a = table.setdefault(k, [0.0, 0.0])
                a[0] += s.score
                a[1] += 1.0

    def mean(i: int, t: Track) -> float:
        a = means[i].get(keys_of[i](t))
        return a[0] / a[1] if a else 0.0

    ranks: Dict[str, int] = {}
    for t in songs:
        s = st(t)
        if s.plays or s.liked:
            continue
        ranks[t.path] = key((2.0 * mean(0, t) + mean(1, t) + mean(2, t)) / 4.0 + s.score)
    discover = sorted((p for p, r in ranks.items() if r > 0), key=lambda p: (-ranks[p], p))
    return {"favourites": favourites, "on_repeat": on_repeat, "rediscover": rediscover, "discover": discover}


def daily_mix(day: str, pools: Dict[str, List[str]], artist_of: Callable[[str], str]) -> List[str]:
    """The Daily Mix of `day` from the mixes' candidates (see the module's text)."""
    counts: Dict[str, int] = {}
    chosen: List[str] = []
    seen = set()

    def take(pool: List[str], quota: int, limit: int) -> None:
        got = 0
        for p in pool:
            if got >= quota:
                break
            k = artist_of(p)
            if p in seen or (k and counts.get(k, 0) >= limit):
                continue
            seen.add(p)
            chosen.append(p)
            got += 1
            if k:
                counts[k] = counts.get(k, 0) + 1

    take(day_order(day, pools["rediscover"][:DAILY_POOL * DAILY_REDISCOVER]), DAILY_REDISCOVER, DAILY_PER_ARTIST)
    take(day_order(day, pools["discover"][:DAILY_POOL * DAILY_DISCOVER]), DAILY_DISCOVER, DAILY_PER_ARTIST)
    rest = DAILY_SIZE - DAILY_REDISCOVER - DAILY_DISCOVER
    favourites = day_order(day, pools["favourites"][:DAILY_POOL * rest])
    take(favourites, DAILY_SIZE - len(chosen), DAILY_PER_ARTIST)
    limit = DAILY_PER_ARTIST
    while len(chosen) < MIN_FILL and any(p not in seen for p in favourites):
        limit += 1
        take(favourites, MIN_FILL - len(chosen), limit)
    return spread_artists(day_order(day, chosen), artist_of)


def mixes(tracks: Iterable[Track], plays: Iterable[Play], liked: Iterable[str] = (), disliked: Iterable[str] = (),
          now: Optional[float] = None, day: Optional[str] = None) -> List[dict]:
    """The mixes: [{"id", "name", "description", "paths"}], in MIXES' order, those with
    fewer than MIN_SONGS songs left out.  `day` ("YYYY-MM-DD", the local date of `now`
    when not given) is the Daily Mix's day."""
    now = time.time() if now is None else float(now)
    day = day_of(now) if day is None else day
    tracks = list(tracks)
    pools = candidates(tracks, plays, liked, disliked, now)
    artists = {t.path: artist_key(t) for t in tracks}

    def artist_of(path: str) -> str:
        return artists.get(path, "")

    def limited(name: str, size: int) -> List[str]:
        return spread_artists(limit_artists(pools[name], artist_of, size, MAX_PER_ARTIST), artist_of)

    paths = {
        "favourites": limited("favourites", FAVOURITES_SIZE),
        "on_repeat": limited("on_repeat", ON_REPEAT_SIZE),
        "rediscover": limited("rediscover", REDISCOVER_SIZE),
        "discover": limited("discover", DISCOVER_SIZE),
        "daily": daily_mix(day, pools, artist_of),
    }
    return [{"id": mid, "name": name, "description": text, "paths": paths[mid]}
            for mid, name, text in MIXES if len(paths[mid]) >= MIN_SONGS]


def mix_names() -> List[str]:
    return [m[1] for m in MIXES]
