# CLAUDE.md

This file guides Claude (and other contributors) working in this repository.

## Standing instruction: keep the docs current

Every change to behaviour, options, thresholds, verdicts, file layout or dependencies must update **both** `README.md` (the user-facing documentation) and this `CLAUDE.md` (the contributor/agent notes) in the same commit. If you add a CLI flag, document it in the README options table. If you change a detection threshold, update the "How detection works" section of the README and the tunables table below. Add a line to the changelog at the bottom of this file for each notable change.

## What the project does

`realhires` is a Python CLI that scans audio files or folders and classifies each file as REAL HI-RES, FAKE HI-RES, SUSPICIOUS, NOT HI-RES, LOSSY FORMAT or ERROR. It decodes the audio with FFmpeg and inspects the samples, so it never trusts tags or headers.

## Commands

| Task | Command |
| --- | --- |
| Run the test suite (about 6 s, it synthesises its own audio with ffmpeg) | `python3 -m pytest -q` |
| Run the CLI from the source tree | `python3 -m realhires PATH... [-v] [--json F] [--csv F] [--plot DIR]` |
| Install the `realhires` command | `pip install .` |

The runtime requirements are Python 3.9 or newer, NumPy, and `ffmpeg`/`ffprobe` on `PATH`. There are no other runtime dependencies, and plots are hand-written SVG, so matplotlib is not needed. Keep it that way unless there is a strong reason.

## Layout

| File | Role |
| --- | --- |
| `realhires/probe.py` | Calls `ffprobe` to build `AudioInfo` (codec, rate, claimed bits, float/lossy flags). `iter_pcm()` streams decoded audio from `ffmpeg` as left-justified int32 `(frames, channels)` blocks. `LOSSY_CODECS` lives here. |
| `realhires/analysis.py` | Contains `SpectrumAccumulator` (streaming Welch spectrum, Blackman-Harris window, about 6 Hz bins), `BitDepthAccumulator` (lowest-set-bit histogram), and `analyze_spectrum()`, which computes the bandwidth, the cliff test at candidate source Nyquists, and the lossy cliff scan. All detection tunables are module constants. |
| `realhires/verdict.py` | Contains `analyze_file()` (the per-file pipeline, which never raises and turns failures into ERROR), `_judge()` (measurements to verdict plus human-readable reasons), and `FileResult` (with `to_dict()` for JSON/CSV). |
| `realhires/report.py` | Terminal table rows and colours, summary line, JSON/CSV writers, and the dependency-free SVG spectrum plot `write_svg()`. |
| `realhires/cli.py` | Handles argparse, file collection (`AUDIO_EXTENSIONS`), the `ProcessPoolExecutor` fan-out (`-j`), and ordered streaming output. |
| `tests/conftest.py` | `music_like()` synthesises pink-ish stereo noise. The session fixture `library` builds real and fake files with ffmpeg (soxr and swr upsampling, s16 to s24/f32 padding, MP3 transcode, dull, silence). |
| `tests/test_detection.py` | End-to-end verdict tests plus a CLI smoke test with all exports. |

## Detection tunables (`realhires/analysis.py`)

| Constant | Value | Meaning |
| --- | --- | --- |
| `CLIFF_DB` / `CLIFF_STRONG_DB` | 25 / 40 dB | The level drop from 88–96 % to 102–112 % of a candidate source Nyquist that means "upsampled", at medium and high confidence. |
| `STEEP_DROP_DB` | 20 dB | The drop across the bandwidth edge that counts as a brick wall, used by the fallback upsample rule. |
| `LOSSY_CLIFF_DB`, `LOSSY_MAX_HZ` | 25 dB, 20.3 kHz | The lossy-encoder low-pass search, run over 11–20.3 kHz. |
| `CONTENT_MARGIN_DB` | 10 dB | How far content must sit above the floor reference. |
| `RELEVANT_RANGE_DB` | 110 dB | Content further below the spectral peak than this is ignored, which drops resampler images. |
| `ULTRASONIC_HZ` | 24.5 kHz | A high-rate file with bandwidth below this and no cliff is SUSPICIOUS. |
| `BitDepthAccumulator.effective_bits(threshold=0.005)` | 0.5 % | The share of samples needed for a bit position to count as used. |

Candidate source rates are limited to `rate * 1.5 <= file_rate`, so a 96 kHz file is never accused of being upsampled from 88.2 kHz.

## Conventions and gotchas

- ffmpeg output is always `-f s32le -acodec pcm_s32le`. The bit-depth test relies on ffmpeg left-justifying integer samples, and on float-to-s32 conversion being undithered, which is the swresample default.
- Spectrum levels are per bin and normalised so that a full-scale sine reads 0 dB. They are therefore much lower (about -60 dB for music, about -180 dB for a 24-bit floor at a 16k FFT) than the levels shown by spectrum viewers with small FFTs. `quantization_floor_db()` gives the theoretical floor.
- `analyze_file()` must never raise. Any exception becomes `verdict=ERROR` with the message in `reasons`.
- `FileResult.spectrum` holds large arrays. It is only kept when `--plot` is used and is cleared in the CLI after plotting. Anything that the estimated-resolution logic needs must be copied onto `FileResult` fields, never read from `spectrum`.
- The tests must stay self-contained. Generate audio in fixtures instead of committing audio files.
- Always use full, specific sentences in user-facing reasons, because they are printed verbatim.

## Changelog

- 1.0.0 (2026-11-19): Initial release. Added the spectral cliff test, the lossy cliff scan, effective bandwidth, the bit-padding test (integer and float), the CLI with JSON/CSV/SVG output and parallel jobs, and the synthetic test suite.
