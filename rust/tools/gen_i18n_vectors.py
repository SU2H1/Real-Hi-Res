"""Write rust/crates/core/tests/python_i18n.json: how realhires.i18n translates
sample texts into Japanese (exact texts, patterns with numbers, "; " parts,
sentences, translated values inside patterns), so the Rust port is checked to
match (2.1; the core texts of 2.2 too).  Run it after changing realhires/i18n.py or the catalog:

    python3 tests/i18n_keys.py --merge && python3 rust/tools/gen_i18n_vectors.py
"""

import json
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, ROOT)

from realhires import i18n  # noqa: E402

TEXTS = [
    "Library", "Hi-Res Lossless", "MP3 320 kbps", "6 channels", "24-bit", "32-bit float", "—",
    "Graphic: Rock", "Graphic: Rock, parametric: 3 bands (pre-amp −4.5 dB)", "Parametric: 1 band",
    "From 96 kHz to 48 kHz (soxr)", "96 kHz, 32-bit float, stereo", "9 PM", "12 AM", "28 May 2026",
    "About 3 days of music.", "Radiohead · 12 plays", "1 play, 4 minutes", "35% of your listening time",
    "by 12 artists", "5 days", "1 day", "3 albums", "about 1 h 20 min left", "about 6 min left",
    "estimating the time left", "less than a minute left", "Fixable to 16/44.1", "16/44.1, stays fake: MP3/AAC source",
    "Upsampled from 44.1 kHz: nothing above 22.05 kHz; Padded: only 16 of 24 bits used",
    "Upsampled from 48 kHz: nothing above 24 kHz; Something new the catalogue does not know",
    "Genuine 24-bit at standard 44.1 kHz: hi-res by bit depth only", "Genuine 24-bit at 32 kHz: hi-res by bit depth only",
    "Spectrum collapses by 87 dB around 22.05 kHz (level is down 6 dB at 21.5 kHz and down 20 dB by 22.0 kHz), the "
    "Nyquist limit of a 44.1 kHz master: upsampled from 44.1 kHz (high confidence). Still hi-res, but not the "
    "advertised 192 kHz.",
    "Every sample lies on an even grid with a step of 4.0 levels of the 24-bit scale instead of 1. That is what a "
    "16-bit master gives after a gain change of -1.5 dB (with or without dither) on its way into a 24-bit file, so the "
    "low bits hold no real detail and the resolution is still 16 bits.",
    "MusicBrainz filled in the information of 5 songs. 2 could not be looked up: No answer from musicbrainz.org after "
    "4 attempts (the server asked to slow down).",
    "A sentence. Another one that is not known. 3 more.", "", "Unknown Album", "Rock", "Kind of Blue",
    "{n} songs", "x; y", "Off",
    # 2.2: bit-perfect playback, the Quality tooltip, the stats calendar, the Cover window, renaming, lyrics labels
    "No: The device cannot play 96 kHz, so the song is resampled to 48 kHz. The device could not be opened for this "
    "app alone (this system has no way to do that), so the sound is mixed by the system. The device has no 24-bit "
    "format, so the samples are converted.",
    "No: The device is in use by another program, so the sound is mixed by the system. The 64-bit float samples are "
    "played as 32-bit float.",
    "No: This 6-channel song is mixed down to stereo.", "On",
    "Yes: 24-bit, 96 kHz, exclusive mode (WASAPI)",
    "Yes (decoded lossy audio): 32-bit float, 48 kHz, the device runs at 48 kHz (CoreAudio)",
    "Yes: 16-bit, 44.1 kHz, ALSA hardware device hw:1", "96 kHz, 24-bit integer in 32-bit, stereo",
    "24-bit/96 kHz", "32-bit float/44.1 kHz", "MP3 · 320 kbps · 44.1 kHz",
    "Tuesday 24 November 2026", "November 2026", "2 songs played, 42 minutes", "1 song played, 1 minute", "<1 min",
    "1,250 min", "No music on this day.",
    "1,200 × 1,200 pixels · JPEG · 245.3 KB", "Unknown format · 812 bytes", "PNG · 4.8 MB", "Inside 01 Song.flac",
    "cover.jpg in the album's folder", "Changed the cover of 1 of 3 songs.", "2 of the songs had no cover inside.",
    "b.wav had no cover inside.", "Could not save cover.png in the album's folder: [Errno 13] Permission denied",
    "cover.jpg in the album's folder was not deleted; it is shown for the songs that have no cover inside.",
    "Could not remove the cover of a.m4a: FFmpeg could not copy the file with the new cover: Invalid argument.",
    "Could not save the information of a.flac: FFmpeg's copy did not have exactly the same audio as the original, "
    "so the file was left unchanged.",
    "The genre is too long: at most 1,000 characters fit.", "Write the year as four digits, such as 2004.",
    "Already has lyrics (a.lrc).", "Saved synced lyrics as a.lrc and embedded them in a.flac.",
    "Lyrics (not synced)", "inside the file",
    "Could not change the cover of a.ogg: Only Ogg files with Vorbis or Opus audio can be changed directly.",
    "The MP3 file has an ID3v2.2 tag, which cannot be updated.",
]


def main():
    i18n.reset()
    i18n.set_language("日本語")
    cases = [{"text": t, "tr_text": i18n.tr_text(t), "tr": i18n.tr(t), "tr_name": i18n.tr_name(t)} for t in TEXTS]
    templates = [["{a}, {b}", "x, y, z"], ["{d:num} {m} {y:num}", "28 May 2026"], ["{d:num} {m} {y:num}", "From 1 to 2"],
                 ["{n:num} days", "−5 days"], ["{n:num} days", "five days"], ["a{x}b{y}c", "aXbYbZc"], ["{x}", ""]]
    matches = [[t, s, i18n.match_template(t, s)] for t, s in templates]
    i18n.set_language("English")
    path = os.path.join(ROOT, "rust", "crates", "core", "tests", "python_i18n.json")
    with open(path, "w", encoding="utf-8", newline="\n") as fh:
        json.dump({"cases": cases, "matches": matches}, fh, ensure_ascii=False, indent=1)
        fh.write("\n")
    print(f"{path}: {len(cases)} texts")


if __name__ == "__main__":
    main()
