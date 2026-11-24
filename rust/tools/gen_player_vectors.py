"""Writes the Python answers the Rust player core is tested against:
rust/crates/core/tests/python_eq.json, python_queue.json, python_stats.json,
python_eqfiles.json, python_themes.json, python_playerstate.json and
python_tagedit.json (the tag editor and the themes folder).  Run from
the repository root:

    python3 rust/tools/gen_player_vectors.py
"""
import json
import os
import sys

sys.path.insert(0, os.getcwd())
import time  # noqa: E402

from realhires import eq, musiclib, playerstate, stats, tagedit, themes  # noqa: E402
from realhires.musiclib import Track  # noqa: E402
from realhires.playqueue import PlayQueue, Song, SplitMix64, shuffled  # noqa: E402

OUT = os.path.join("rust", "crates", "core", "tests")

cases = []
for rate in (44100.0, 48000.0, 96000.0):
    for kind in eq.BAND_TYPES:
        for freq, gain, q in ((32.0, 6.0, 1.41), (1000.0, -7.5, 0.7), (12000.0, 3.25, 4.0), (30000.0, 6, 1)):
            cases.append({"kind": kind, "freq": freq, "gain": gain, "q": q, "rate": rate,
                          "coeffs": list(eq.biquad(kind, freq, gain, q, rate))})
rock = eq.apply_preset(eq.default_state(), "Rock")
rock["enabled"] = True
mixed = eq.apply_preset(eq.default_state(), "Vocal")
mixed.update({"enabled": True, "prevent_clipping": False})
mixed["graphic"]["preamp"] = -2.5
mixed["parametric"] = {"enabled": True, "preamp": 1.5, "bands": [
    {"type": "low_shelf", "freq": 80.0, "gain": 3.0, "q": 0.707, "on": True},
    {"type": "notch", "freq": 60.0, "gain": 0.0, "q": 8.0, "on": True},
    {"type": "peak", "freq": 2500.0, "gain": -2.0, "q": 2.0, "on": False}]}
broken = {"enabled": True, "graphic": {"gains": [99, -99, "x"], "preamp": "y"},
          "parametric": {"enabled": True, "bands": [{"type": "?", "freq": 1e9, "q": -1}]}}
states = []
for st in (eq.default_state(), rock, mixed, broken):
    states.append({"state": st, "normalized": eq.normalize(st), "preamp": eq.preamp_db(st),
                   "stages_44100": [list(c) for c in eq.stages(st, 44100.0)],
                   "response": [[f, db] for f, db in eq.response(st)][::20]})
with open(os.path.join(OUT, "python_eq.json"), "w", encoding="utf-8") as fh:
    json.dump({"biquads": cases, "states": states, "presets": eq.PRESETS}, fh, indent=0)

songs = []
for a, (artist, genre) in enumerate((("Band A", "Rock"), ("Band B", "Jazz"), ("Band C", ""))):
    for album in range(2):
        for n in range(1, 5):
            songs.append(Song(f"/m/{artist}/{album}/{n:02d}.flac", f"Song {a}{album}{n}", artist,
                              f"{artist} Album {album}", artist if a != 2 else "", genre if album == 0 else "Pop",
                              1 if n < 4 else 2, n))
everything = [s.path for s in songs]
runs = []
for shuffle in ("off", "songs", "library", "genre", "artist", "album"):
    for repeat in ("off", "song", "album", "stop"):
        for seed in (1, 99):
            q = PlayQueue(songs, everything[:10], shuffle=shuffle, repeat=repeat, seed=seed)
            steps = [q.start(everything[2])]
            for op in ("auto", "next", "auto", "prev", "next", "auto", "next", "next", "auto", "next", "auto",
                       "set_shuffle:library", "next", "set_repeat:album", "auto", "auto", "next"):
                if op == "auto":
                    steps.append(q.next(auto=True))
                elif op == "next":
                    steps.append(q.next())
                elif op == "prev":
                    steps.append(q.previous())
                elif op.startswith("set_shuffle:"):
                    q.set_shuffle(op.split(":")[1])
                    steps.append(q.current)
                else:
                    q.set_repeat(op.split(":")[1])
                    steps.append(q.current)
            runs.append({"shuffle": shuffle, "repeat": repeat, "seed": seed, "steps": steps,
                         "up_next": q.up_next(30)})
# disliked songs are left out of shuffled orders
for shuffle in ("off", "songs", "library", "artist"):
    for seed in (5, 77):
        disliked = [everything[1], everything[5], everything[11], everything[17]]
        q = PlayQueue(songs, everything[:10], shuffle=shuffle, repeat="off", seed=seed, disliked=set(disliked))
        steps = [q.start(everything[2])]
        for op in ("auto", "next", "auto", "prev", "next", "auto", "next", "next", "auto", "next", "auto",
                   "set_shuffle:library", "next", "set_repeat:album", "auto", "auto", "next"):
            if op == "auto":
                steps.append(q.next(auto=True))
            elif op == "next":
                steps.append(q.next())
            elif op == "prev":
                steps.append(q.previous())
            elif op.startswith("set_shuffle:"):
                q.set_shuffle(op.split(":")[1])
                steps.append(q.current)
            else:
                q.set_repeat(op.split(":")[1])
                steps.append(q.current)
        runs.append({"shuffle": shuffle, "repeat": "off", "seed": seed, "steps": steps, "up_next": q.up_next(30),
                     "disliked": disliked})
rng = SplitMix64(2024)
with open(os.path.join(OUT, "python_queue.json"), "w", encoding="utf-8") as fh:
    json.dump({"songs": [s.__dict__ for s in songs], "context": everything[:10], "start": everything[2],
               "ops": ["auto", "next", "auto", "prev", "next", "auto", "next", "next", "auto", "next", "auto",
                       "set_shuffle:library", "next", "set_repeat:album", "auto", "auto", "next"],
               "runs": runs, "splitmix_2024": [str(rng.next()) for _ in range(5)],
               "shuffled_9": shuffled([str(i) for i in range(12)], SplitMix64(9))}, fh, indent=0)

# ------------------------------------------------------------- statistics ---
def tokyo(ts):
    return time.gmtime(ts + 9 * 3600)


tracks = []
artists_ = ["Mira Calloway", "The Northern Quay", "Juniper Vale", "", "mira calloway"]
for i in range(14):
    a = artists_[i % len(artists_)]
    tracks.append(Track(path=f"/lib/{'AB'[i % 2]}/{'sub/' if i % 3 == 0 else ''}{i:02d}.flac", title=f"Song {i}",
                        artist=a, album=["Paper Lanterns", "Harbour Lights", "", "Static Bloom"][i % 4],
                        album_artist="Various" if i % 5 == 0 else "", genre=["Pop", "jazz", "", "Jazz"][i % 4],
                        composer=["", "Zed", "zed", "Ann"][i % 4], year=["2001", "1999", "x", "", "2024"][i % 5],
                        track=i % 6, disc=1 + i % 2, duration=[20.0, 180.0, 240.5, 61.0][i % 4],
                        codec=["flac", "mp3", "alac", "aac"][i % 4], is_lossy=i % 4 in (1, 3),
                        sample_rate=[44100, 48000, 96000, 192000][i % 4], bits=[16, 0, 24, 0][i % 4],
                        is_float=i == 6, size=1000 * i, mtime_ns=i))
plays = []
t = 1_700_000_000.0
for i in range(60):
    tr = tracks[(i * 7) % len(tracks)]
    t += [600.0, 3600.0 * 5, 86400.0, 86400.0 * 3, 1800.0][i % 5]
    secs = [tr.duration, 5.0, 31.0, tr.duration / 2.0, 0.0 + 12.5][i % 5]
    plays.append(stats.Play(tr.path, t, secs, tr.duration, tr.display_title, tr.artist, tr.album, tr.album_artist,
                            tr.genre, tr.year, tr.composer))
st = stats.compute(plays, localtime=tokyo)
half = stats.compute(plays[:7], localtime=tokyo)
groups = {name: [[g["name"], [x.path for x in g["tracks"]]] for g in fn(tracks)]
          for name, fn in (("artists", musiclib.artists), ("album_artists", musiclib.album_artists),
                           ("genres", musiclib.genres), ("composers", musiclib.composers), ("years", musiclib.years))}
folders = {f: [[list(x) for x in musiclib.folder_view(tracks, f)[0]], [x.path for x in musiclib.folder_view(tracks, f)[1]]]
           for f in ("/lib", "/lib/A", "/lib/B", "/lib/A/sub", "/nope")}
ls = stats.library_stats(tracks)
with open(os.path.join(OUT, "python_stats.json"), "w", encoding="utf-8") as fh:
    json.dump({"tracks": [x.to_json() for x in tracks], "plays": [p.__dict__ for p in plays], "offset": 9 * 3600,
               "stats": st, "cards": stats.cards(st), "stats_7": half, "cards_7": stats.cards(half),
               "cards_empty": stats.cards(stats.compute([], localtime=tokyo)), "groups": groups, "folders": folders,
               "library": ls, "hours": [stats.hour_label(h) for h in range(24)]}, fh, indent=0,
              ensure_ascii=False)

# ------------------------------------------------------------- EQ files ---
param = eq.apply_preset(eq.default_state(), "Rock")
param["parametric"] = {"enabled": True, "preamp": -1.5, "bands": [
    {"type": "high_shelf", "freq": 8000, "gain": -3, "q": 0.71, "on": True},
    {"type": "notch", "freq": 3000, "gain": 0, "q": 5, "on": False},
    {"type": "low_pass", "freq": 18000.25, "gain": 2, "q": 0.5, "on": True}]}
exports = [{"name": n, "state": s_, "json": eq.export_json(n, s_), "apo": eq.export_apo(n, s_)}
           for n, s_ in (("My Rock", param), ("Flat", eq.default_state()), ("Für Elise ♪", mixed))]
texts = [
    "Preamp: -6.2 dB\nFilter 1: ON PK Fc 105 Hz Gain 3.5 dB Q 0.70\nFilter 2: ON LSC Fc 105 Hz Gain 4.1 dB Q 0.71\n",
    "\ufeff# AutoEQ\r\nPreamp: -3,5 dB\r\nFilter 1: ON PK Fc 105Hz Gain 3.5dB Q 0.7\r\nFilter 2: OFF HP Fc 20 Hz\r\n"
    "Filter 3: ON PK Fc 1000 Hz Gain -2 dB BW Oct 1.0\r\nFilter: ON NO Fc 60 Hz Q 30\r\nFilter 5: ON BP Fc 1 Hz\r\n",
    "Preamp: -20 dB\n" + "".join(f"Filter {i}: ON PK Fc {100 * i} Hz Gain {i - 6} dB Q 1\n" for i in range(1, 13)),
    "GraphicEQ: 20 -1; 50 2; 100 3; 1000 0; 20000 -6",
    "Preamp: -2 dB\nGraphicEQ: 0 5; 25 -14; 16000 14",
    "Preamp: x dB\nfilter 1 : on pk fc 440 hz gain 1 db q 2\nFilter 2: ON LS Fc 80 Hz Gain 2 dB\n",
    eq.export_json("Saved", param), json.dumps({"name": "  ", "eq": mixed}), json.dumps(mixed),
    json.dumps({"hello": 1}), "hello", "", "Filter 1: ON PK Fc x Hz Gain 1 dB Q 1",
]
imports = []
for text in texts:
    try:
        name, state, notes = eq.import_eq(text, "Imported")
        imports.append({"text": text, "name": name, "state": state, "notes": notes})
    except eq.EqFileError as exc:
        imports.append({"text": text, "error": str(exc)})
with open(os.path.join(OUT, "python_eqfiles.json"), "w", encoding="utf-8") as fh:
    json.dump({"exports": exports, "imports": imports}, fh, indent=0, ensure_ascii=False)

# ---------------------------------------------------------------- themes ---
custom = [themes.normalize({"name": " My  Theme ", "dark": True, "colors": {"accent": "#00FF00", "text": "red"}}),
          themes.normalize({"name": "Paper", "colors": {"background": "#fafafa"}})]
cases_t = [{"input": x, "output": themes.normalize(x)} for x in (
    {"name": " My  Theme ", "dark": True, "colors": {"accent": "#00FF00", "text": "red"}}, {"name": ""}, "x",
    {"name": "Paper", "colors": {"background": "#fafafa", "panel": " #ABCDEF "}}, {"name": "N", "colors": 5})]
imports_t = []
for text in (themes.export_theme(custom[0]), themes.export_theme(dict(custom[1], name="Dark")), "{}", "nope",
             json.dumps({"name": "Paper", "colors": {}})):
    try:
        imports_t.append({"text": text, "theme": themes.import_theme(text, custom)})
    except themes.ThemeError as exc:
        imports_t.append({"text": text, "error": str(exc)})
checks = []
for n in ("dark", "My theme", " New  one ", "", "Match the system"):
    try:
        checks.append([n, themes.check_name(n, custom), None])
    except themes.ThemeError as exc:
        checks.append([n, None, str(exc)])
with open(os.path.join(OUT, "python_themes.json"), "w", encoding="utf-8") as fh:
    json.dump({"builtin": themes.BUILTIN, "custom": custom, "names": themes.names(custom), "cases": cases_t,
               "imports": imports_t, "checks": checks, "copy": [themes.copy_name("Midnight", custom),
                                                               themes.copy_name("My Theme", custom)],
               "export": themes.export_theme(custom[0])}, fh, indent=0, ensure_ascii=False)

# ---------------------------------------------------------- player state ---
messy = {"liked": ["/a", "/b", 5], "disliked": ["/b", "/c", "/c", ""], "volume": "0.4",
         "eq_presets": [{"name": " Night  Mode ", "eq": mixed}, {"name": "night mode", "eq": {}}, {"name": ""}, 7],
         "playlists": [{"id": "a1", "name": "  Road ", "created": "12.5", "songs": ["/a", "/a", 3, "/b"]},
                       {"id": "a1", "name": "dupe"}, {"name": "no id"}, {"id": "b2", "name": "", "songs": "x"}]}
ps = playerstate.PlayerState.from_json(messy)
ops = playerstate.PlayerState()
pid = ops.create_playlist("Road Trip", ["/m/a", "/m/b", "/m/a"], now=1000.0)
pid2 = ops.create_playlist("road trip", now=1000.0)
ops.add_to_playlist(pid, ["/m/b", "/m/c"])
ops.move_in_playlist(pid, "/m/c", -2)
ops.rename_playlist(pid2, "  Night   Drive ")
ops.set_liked("/m/a", True)
ops.set_disliked("/m/a", True)
ops.set_disliked("/m/z", True)
ops.save_eq_preset("  My   Jazz ", eq.apply_preset(eq.default_state(), "Jazz"))
ops.save_eq_preset("Other", eq.default_state())
ops.rename_eq_preset("Other", "Bright")
errors = []
for name in ("rock", "my jazz", "", "  "):
    try:
        ops.check_preset_name(name)
        errors.append([name, None])
    except playerstate.InvalidName as exc:
        errors.append([name, str(exc)])
with open(os.path.join(OUT, "python_playerstate.json"), "w", encoding="utf-8") as fh:
    json.dump({"messy": messy, "messy_out": ps.to_json(), "ops_out": ops.to_json(), "name_errors": errors},
              fh, indent=0, ensure_ascii=False)
print(len(cases), "biquads,", len(states), "states,", len(runs), "queue runs,", len(plays), "plays,",
      len(imports), "EQ files")


# ------------------------------------------------- tag editor, themes folder ---
import hashlib  # noqa: E402
import struct  # noqa: E402
import tempfile  # noqa: E402


def flac_file(comments, id3=b"", padding=64, vorbis=True):
    info = b"\x10\x00\x10\x00" + b"\x00" * 6 + b"\x0a\xc4\x42\xf0" + b"\x00" * 4 + b"\x00" * 16
    blocks = [(0, info)]
    if vorbis:
        vendor = b"reference libFLAC 1.4.3"
        body = struct.pack("<I", len(vendor)) + vendor + struct.pack("<I", len(comments))
        for c in comments:
            c = c.encode("utf-8")
            body += struct.pack("<I", len(c)) + c
        blocks.append((4, body))
    blocks.append((6, b"\x00\x00\x00\x03" + b"\x00" * 28))  # a PICTURE-like block that must stay
    if padding:
        blocks.append((1, b"\x00" * padding))
    out = id3 + b"fLaC"
    for i, (k, b) in enumerate(blocks):
        out += bytes([k | (0x80 if i == len(blocks) - 1 else 0)]) + len(b).to_bytes(3, "big") + b
    return out + b"\xff\xf8AUDIO-FRAMES" * 3


def id3_tag(major, frames, padding=32):
    body = b""
    for fid, text in frames:
        if isinstance(text, bytes):
            fb = text
        elif major == 4:
            fb = b"\x03" + text.encode("utf-8")
        else:
            fb = b"\x00" + text.encode("latin-1")
        size = (bytes([(len(fb) >> 21) & 0x7F, (len(fb) >> 14) & 0x7F, (len(fb) >> 7) & 0x7F, len(fb) & 0x7F])
                if major == 4 else struct.pack(">I", len(fb)))
        body += fid + size + b"\x00\x00" + fb
    body += b"\x00" * padding
    n = len(body)
    return b"ID3" + bytes([major, 0, 0]) + bytes([(n >> 21) & 0x7F, (n >> 14) & 0x7F, (n >> 7) & 0x7F, n & 0x7F]) + body


MP3_AUDIO = b"\xff\xfb\x90\x64" + b"MPEG-FRAME" * 4
flacs = {
    "plain": flac_file(["TITLE=Old", "ARTIST=One", "ARTIST=Two", "ALBUM ARTIST=AA", "ALBUMARTIST=AA2",
                        "TRACKNUMBER=2", "TRACKTOTAL=9", "DATE=2001", "LYRICS=[00:01.00] la", "genre=Pop"]),
    "no_padding": flac_file(["TITLE=Old", "DISCNUMBER=1", "TOTALDISCS=2"], padding=0),
    "no_vorbis": flac_file([], vorbis=False),
    "id3_front": flac_file(["TITLE=Old"], id3=id3_tag(3, [(b"TIT2", "Front")])),
}
mp3s = {
    "v23": id3_tag(3, [(b"TIT2", "Old"), (b"TPE1", "Art"), (b"TYER", "2001"), (b"TDAT", "2805"), (b"TRCK", "2/9"),
                       (b"USLT", b"\x00XXX\x00la la")]) + MP3_AUDIO,
    "v24": id3_tag(4, [(b"TIT2", "Old"), (b"TDRC", "2001-02"), (b"TPE2", "AA"), (b"TXXX", b"\x03k\x00v")],
                   padding=0) + MP3_AUDIO,
    "none": MP3_AUDIO,
    "v22": b"ID3\x02\x00\x00\x00\x00\x00\x10" + b"\x00" * 16 + MP3_AUDIO,
}
change_sets = [
    {"title": "Neue Ünïcode 日本", "album_artist": "", "date": "2020-05-28", "composer": "Bach",
     "track": "04/12", "disc": "1", "genre": "Jazz"},
    {"artist": "Solo", "track": "3", "disc": "", "date": "1999"},
    {"album": "  Spaced \n out  ", "date": "", "track": ""},
    {},
]


def written(fn, data, changes):
    try:
        return {"output": fn(data, tagedit.check_changes(changes)).hex()}
    except Exception as exc:  # noqa: BLE001
        return {"error": str(exc)}


tag_cases = []
for kind, files, fn in (("flac", flacs, tagedit.flac_with_tags), ("mp3", mp3s, tagedit.mp3_with_tags)):
    for name, data in files.items():
        for ch in change_sets:
            tag_cases.append(dict(kind=kind, name=name, input=data.hex(), changes=ch, **written(fn, data, ch)))
checks_tag = []
for ch in change_sets + [{"track": "x"}, {"disc": "1/x"}, {"date": "May 2004"}, {"title": "x" * 1001},
                         {"track": " 3 / 12 "}, {"track": "007"}, {"unknown": "1", "genre": "\tRock\t"}]:
    try:
        checks_tag.append({"changes": ch, "ok": list(tagedit.check_changes(ch).items())})
    except tagedit.TagError as exc:
        checks_tag.append({"changes": ch, "error": str(exc)})
commands = [{"src": src, "dst": "/tmp/out" + os.path.splitext(src)[1], "changes": ch,
             "args": tagedit.ffmpeg_args(src, "/tmp/out" + os.path.splitext(src)[1], tagedit.check_changes(ch))}
            for src in ("/m/a.m4a", "/m/b.OGG", "/m/c.aiff", "/m/d.wav", "/m/e.opus") for ch in change_sets[:2]]
cannot = {p: tagedit.cannot_edit(p) for p in ("a.ape", "b.DSF", "c.flac", "d.m4a", "e.tta")}
notes = []
for ch, got in ((change_sets[0], {"title": "Neue Ünïcode 日本", "track": "4/12", "date": "2020-05-28", "genre": "Jazz"}),
                (change_sets[0], {"title": "Neue Ünïcode 日本", "album_artist": "x", "date": "2020", "genre": "Jazz",
                                  "composer": "Bach", "track": "4", "disc": "1"}),
                (change_sets[1], {"artist": "Solo", "track": "3/9", "date": "1999"})):
    notes.append({"changes": tagedit.check_changes(ch), "tags": got, "note": tagedit.kept_note("x.wav", tagedit.check_changes(ch), got)})
commons = [[{"title": "A", "album": "X"}, {"title": "B", "album": " X "}], [{"genre": "Pop"}], []]
outcomes = [[tagedit.TagOutcome("a", True, ""), tagedit.TagOutcome("b", True, "b was saved, but x.")],
            [tagedit.TagOutcome("a", True, ""), tagedit.TagOutcome("b", False, "Could not."),
             tagedit.TagOutcome("c", False, "Could not.")],
            [tagedit.TagOutcome("a", False, "No.")], [tagedit.TagOutcome("a", True)]]
own = [themes.normalize({"name": "Neon", "dark": True, "colors": {}})]
folder_files = {"b.json": themes.export_theme(dict(themes.BUILTIN["Dark"], name="Dark")),
                "a.JSON": json.dumps({"name": "Neon", "dark": False, "colors": {"accent": "#00FF00"}}),
                "c.json": "nope", ".hidden.json": themes.export_theme(own[0]), "d.txt": themes.export_theme(own[0]),
                "e.json": "﻿" + themes.export_theme({"name": "Sunset", "dark": False,
                                                         "colors": {"accent": "#ff8800"}}),
                "f.json": themes.export_theme({"name": "Neon", "dark": True, "colors": {}})}
with tempfile.TemporaryDirectory() as d:
    for n, text in folder_files.items():
        with open(os.path.join(d, n), "w", encoding="utf-8") as fh:
            fh.write(text)
    loaded = [{"theme": {k: v for k, v in t.items() if k != "file"}, "file": os.path.basename(t["file"])}
              for t in themes.load_folder(own, d)]


def ogg_file(first, header_packets, serial=0x1234ABCD, audio_pages=3, other_serial=False):
    """An Ogg file laid out as FFmpeg writes one: the identification header on the
    first page, every other header packet on pages of its own, then audio pages."""
    out = tagedit._ogg_page(2, b"\x00" * 8, serial, 0, [len(first)], first)
    seq = 1
    for pkt in header_packets:
        pages = tagedit._ogg_pages_for([pkt], serial, seq)
        out += b"".join(pages)
        seq += len(pages)
    if other_serial:
        out += tagedit._ogg_page(2, b"\x00" * 8, serial + 1, 0, [5], b"OTHER")
    for i in range(audio_pages):
        body = bytes((i * 7 + k) % 256 for k in range(300))
        out += tagedit._ogg_page(4 if i == audio_pages - 1 else 0, struct.pack("<q", 1024 * (i + 1)), serial,
                                 seq, [255, 45], body)
        seq += 1
    return out


def vorbis_comment(magic, comments, tail):
    vendor = b"Lavf62.0.100"
    body = struct.pack("<I", len(vendor)) + vendor + struct.pack("<I", len(comments))
    for c in comments:
        c = c.encode("utf-8")
        body += struct.pack("<I", len(c)) + c
    return magic + body + tail


VORBIS_ID = b"\x01vorbis" + b"\x00\x00\x00\x00\x02\x44\xac\x00\x00" + b"\x00" * 12 + b"\xb8\x01"
OPUS_HEAD = b"OpusHead\x01\x02\x38\x01\x80\xbb\x00\x00\x00\x00\x00"
SETUP = b"\x05vorbis" + bytes(range(256)) * 12
small = ["TITLE=Old", "ARTIST=One", "ALBUMARTIST=AA", "TRACKNUMBER=2", "TRACKTOTAL=9", "DATE=2001"]
oggs = {
    "vorbis": ogg_file(VORBIS_ID, [vorbis_comment(b"\x03vorbis", small, b"\x01"), SETUP]),
    "vorbis_big": ogg_file(VORBIS_ID, [vorbis_comment(b"\x03vorbis", small + ["COMMENT=" + "z" * 70000],
                                                      b"\x01"), SETUP]),
    "opus": ogg_file(OPUS_HEAD, [vorbis_comment(b"OpusTags", small, b"\x00\x00padding")]),
    "speex": ogg_file(b"Speex   1.2" + b"\x00" * 69, [vorbis_comment(b"", small, b"")]),
    "two_streams": ogg_file(VORBIS_ID, [vorbis_comment(b"\x03vorbis", small, b"\x01")], other_serial=True),
    "not_ogg": b"RIFF....WAVEfmt ",
}
ogg_cases = []
for name, data in oggs.items():
    for ch in change_sets:
        case = {"name": name, "changes": ch}
        try:
            out = tagedit.ogg_with_tags(data, tagedit.check_changes(ch))
            case.update(sha1=hashlib.sha1(out).hexdigest(), size=len(out))
            if name == "vorbis":
                case["output"] = out.hex()
        except tagedit.TagError as exc:
            case["error"] = str(exc)
        ogg_cases.append(case)
crcs = {x: tagedit.ogg_crc(x.encode()) for x in ("", "123456789", "OggS")}

with open(os.path.join(OUT, "python_tagedit.json"), "w", encoding="utf-8") as fh:
    json.dump({"cases": tag_cases, "checks": checks_tag, "commands": commands, "cannot": cannot, "notes": notes,
               "commons": [{"tags": c, "values": tagedit.common_values(c)} for c in commons],
               "summaries": [{"outcomes": [[o.path, o.ok, o.message] for o in oc],
                              "summary": tagedit.save_summary(oc)} for oc in outcomes],
               "folder": {"own": own, "files": folder_files, "loaded": loaded},
               "ogg_inputs": {name: data.hex() for name, data in oggs.items()}, "ogg": ogg_cases,
               "crc": crcs}, fh, indent=0, ensure_ascii=False)
