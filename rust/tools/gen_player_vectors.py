"""Writes the Python answers the Rust player core is tested against:
rust/crates/core/tests/python_eq.json and python_queue.json.  Run from the
repository root:

    python3 rust/tools/gen_player_vectors.py
"""
import json
import os
import sys

sys.path.insert(0, os.getcwd())
from realhires import eq  # noqa: E402
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
rng = SplitMix64(2024)
with open(os.path.join(OUT, "python_queue.json"), "w", encoding="utf-8") as fh:
    json.dump({"songs": [s.__dict__ for s in songs], "context": everything[:10], "start": everything[2],
               "ops": ["auto", "next", "auto", "prev", "next", "auto", "next", "next", "auto", "next", "auto",
                       "set_shuffle:library", "next", "set_repeat:album", "auto", "auto", "next"],
               "runs": runs, "splitmix_2024": [str(rng.next()) for _ in range(5)],
               "shuffled_9": shuffled([str(i) for i in range(12)], SplitMix64(9))}, fh, indent=0)
print(len(cases), "biquads,", len(states), "states,", len(runs), "queue runs")
