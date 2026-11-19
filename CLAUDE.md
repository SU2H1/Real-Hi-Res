# CLAUDE.md

This file guides Claude (and other contributors) working in this repository.

## Standing instruction: keep the docs current

Every change to behaviour, options, thresholds, verdicts, file layout, packaging or dependencies must update **both** `README.md` (the user-facing documentation) and this `CLAUDE.md` (the contributor/agent notes) in the same commit. If you add a CLI flag, document it in the README options table. If you change a detection threshold, update the "How detection works" section of the README and the tunables table below. If you change the GUI, check that the README "Desktop app" section and `docs/gui-screenshot.png` still match it. If you change packaging, update the README "Download" and "Building the packages" sections and `packaging/README-RELEASE.txt`. Add a line to the changelog at the bottom of this file for each notable change, and bump `__version__` in `realhires/__init__.py` and `version` in `pyproject.toml` together.

## What the project does

`realhires` scans audio files or folders and classifies each file as REAL HI-RES, FAKE HI-RES, SUSPICIOUS, NOT HI-RES, LOSSY FORMAT or ERROR. It decodes the audio with FFmpeg and inspects the samples, so it never trusts tags or headers. There is a CLI (`realhires`) and a Tkinter desktop app (`realhires-gui`). Both are also shipped as self-contained PyInstaller packages, with FFmpeg bundled, for Windows, macOS and Linux.

## Commands

| Task | Command |
| --- | --- |
| Run the test suite (about 6 s, it synthesises its own audio with ffmpeg) | `python3 -m pytest -q` |
| Run the CLI from the source tree | `python3 -m realhires PATH... [-v] [--json F] [--csv F] [--plot DIR]` |
| Run the GUI from the source tree (needs Tkinter) | `python3 -m realhires.gui [PATH...]`. Passing paths starts a scan automatically. |
| Install the `realhires` and `realhires-gui` commands | `pip install .` |
| Build a release package locally | `python packaging/fetch_ffmpeg.py && pyinstaller --noconfirm packaging/realhires.spec` |
| Smoke-test a built package | `python packaging/smoke_test.py dist/RealHiRes/realhires-cli` |
| Publish a release | Push a tag `vX.Y.Z`. CI builds the three packages and creates the GitHub Release. |

The runtime requirements are Python 3.9 or newer, NumPy, and `ffmpeg`/`ffprobe` (bundled or on `PATH`). The GUI additionally needs Tkinter (standard library). There are no other runtime dependencies, and plots are hand-written SVG or Tk canvas, so matplotlib is not needed. Keep it that way unless there is a strong reason.

## Layout

| File | Role |
| --- | --- |
| `realhires/probe.py` | Contains `find_tool()`/`tool()` (FFmpeg lookup order: `$REALHIRES_FFMPEG_DIR`, then the PyInstaller bundle `_MEIPASS/ffmpeg`, then `ffmpeg/` next to the executable, then `PATH`), `probe()` (ffprobe to `AudioInfo`), and `iter_pcm()`, which streams decoded audio as left-justified int32 `(frames, channels)` blocks. `LOSSY_CODECS` lives here. `_no_window()` suppresses console windows on Windows. |
| `realhires/analysis.py` | Contains `SpectrumAccumulator` (streaming Welch spectrum, Blackman-Harris window, about 6 Hz bins), `BitDepthAccumulator` (lowest-set-bit histogram), `analyze_spectrum()` (bandwidth, cliff test with the edge tie-break, lossy cliff scan), and `compact_spectrum()` (a 900-point max-hold spectrum for the GUI). All detection tunables are module constants. |
| `realhires/verdict.py` | Contains `analyze_file()` (the per-file pipeline, which never raises and turns failures into ERROR; `keep_spectrum` is for SVG and `keep_plot` is for the GUI), `_judge()` (measurements to verdict plus reasons), and `FileResult` (with `to_dict()` for JSON/CSV). |
| `realhires/report.py` | Terminal table, summary, JSON/CSV writers, and the dependency-free SVG spectrum plot. |
| `realhires/cli.py` | Handles argparse, file collection (`AUDIO_EXTENSIONS`, `collect()`), the `ProcessPoolExecutor` fan-out, and ordered streaming output. |
| `realhires/gui.py` | The Tkinter app. The `Scanner` thread uses a spawn-context `ProcessPoolExecutor` and a `queue` that `App._poll()` drains every 100 ms. Its pieces are a `ttk.Treeview` result list (default filter: Fake plus Suspicious), filter checkboxes with counts, search, sortable columns, a details text panel, a `SpectrumCanvas`, and export/reveal/copy actions. |
| `packaging/realhires.spec` | PyInstaller spec. It builds one folder `RealHiRes/` holding the `RealHiRes` GUI (windowed) and the `realhires-cli` console executable, which share one runtime, with FFmpeg in `_internal/ffmpeg/`. On macOS it is wrapped as `RealHiRes.app`, with the CLI at `Contents/MacOS/realhires-cli`. |
| `packaging/entry_gui.py`, `packaging/entry_cli.py` | Frozen entry points. Both call `multiprocessing.freeze_support()`. |
| `packaging/fetch_ffmpeg.py` | Downloads static ffmpeg and ffprobe into `build/ffmpeg`: BtbN LGPL builds for Windows and Linux x64, Martin Riedl builds for macOS. It also writes `FFMPEG-NOTICE.txt`. |
| `packaging/smoke_test.py` | Runs a frozen CLI with an empty `PATH` on three generated WAVs and checks the verdicts. |
| `packaging/README-RELEASE.txt`, `packaging/RELEASE_NOTES.md` | The README copied into each package, and the body of the GitHub Release. |
| `.github/workflows/build.yml` | On every push or PR, a matrix job (windows-latest, macos-latest arm64, ubuntu-22.04) fetches FFmpeg, runs pytest with that FFmpeg, builds, smoke-tests, packages and uploads artifacts. On a `v*` tag, the `release` job publishes them with softprops/action-gh-release. |
| `tests/conftest.py` | `music_like()` synthesises pink-ish stereo noise. The session fixture `library` builds real and fake files with ffmpeg. `hq_resample()` falls back from soxr to a long swr filter, and MP3 fixtures are omitted (and their tests skipped) when the FFmpeg build has no MP3 encoder. |
| `tests/test_detection.py` | End-to-end verdict tests plus a CLI smoke test with all exports. |
| `docs/gui-screenshot.png` | The screenshot shown in the README. |

## Detection tunables (`realhires/analysis.py`)

| Constant | Value | Meaning |
| --- | --- | --- |
| `CLIFF_DB` / `CLIFF_STRONG_DB` | 25 / 40 dB | The level drop from 88–96 % to 102–112 % of a candidate source Nyquist that means "upsampled", at medium and high confidence. |
| `EDGE_RATIO` | 0.98 | When several candidates pass `CLIFF_DB`, pick the one whose Nyquist best matches (−20 dB point ÷ Nyquist ≈ 0.98). |
| `STEEP_DROP_DB` | 20 dB | The drop across the bandwidth edge that counts as a brick wall, used by the fallback upsample rule. |
| `LOSSY_CLIFF_DB`, `LOSSY_MAX_HZ` | 25 dB, 20.3 kHz | The lossy-encoder low-pass search, run over 11–20.3 kHz. |
| `CONTENT_MARGIN_DB` | 10 dB | How far content must sit above the floor reference. |
| `RELEVANT_RANGE_DB` | 110 dB | Content further below the spectral peak than this is ignored, which drops resampler images. |
| `ULTRASONIC_HZ` | 24.5 kHz | A high-rate file with bandwidth below this and no cliff is SUSPICIOUS. |
| `BitDepthAccumulator.effective_bits(threshold=0.005)` | 0.5 % | The share of samples needed for a bit position to count as used. |

Candidate source rates are limited to `rate * 1.5 <= file_rate`, so a 96 kHz file is never accused of being upsampled from 88.2 kHz.

## Conventions and gotchas

- ffmpeg output is always `-f s32le -acodec pcm_s32le`. The bit-depth test relies on ffmpeg left-justifying integer samples, and on float-to-s32 conversion being undithered, which is the swresample default.
- Always call FFmpeg through `probe.tool("ffmpeg")` / `tool("ffprobe")` with `**_no_window()`, never through a bare `"ffmpeg"` string, or the packaged builds break.
- Spectrum levels are per bin and normalised so that a full-scale sine reads 0 dB. They are therefore much lower (about -60 dB for music, about -180 dB for a 24-bit floor at a 16k FFT) than the levels shown by spectrum viewers with small FFTs. `quantization_floor_db()` gives the theoretical floor.
- `analyze_file()` must never raise. Any exception becomes `verdict=ERROR` with the message in `reasons`.
- `FileResult.spectrum` holds large arrays. It is only kept for `--plot` and is cleared after plotting. `FileResult.plot` is the small GUI version. Anything that the estimated-resolution logic needs must be copied onto `FileResult` fields, never read from `spectrum`.
- GUI worker processes use the **spawn** context, so any script that starts the GUI must guard its code with `if __name__ == "__main__":`. Otherwise every worker re-runs the script and opens another window. Frozen entry points must call `multiprocessing.freeze_support()` first.
- Executable names must not differ only by case (`RealHiRes` vs `realhires`), because Windows and macOS file systems are case-insensitive. That is why the CLI in packages is `realhires-cli`.
- On Linux the GUI uses the `clam` ttk theme, whose progress bar is invisible by default, so `main()` sets an explicit bar colour.
- You can test the GUI headlessly with `xvfb-run` and take screenshots with ImageMagick `import -window root out.png`.
- The tests must stay self-contained. Generate audio in fixtures instead of committing audio files. They must pass with both the distro FFmpeg and the bundled BtbN/Riedl builds used in CI.
- Always use full, specific sentences in user-facing reasons, because they are printed verbatim in the CLI and the GUI.

## Changelog

- 1.1.0 (2026-11-19): Added the Tkinter desktop GUI, with a scrollable list of fake and suspicious files, filters, search, sorting, a details panel with a spectrum, and export. Added self-contained release packages (PyInstaller with bundled FFmpeg) for Windows x64, macOS arm64 and Linux x64, built and published by GitHub Actions on `v*` tags. Added the bundled-FFmpeg lookup and suppressed console windows on Windows. Added the edge-frequency tie-break between neighbouring source rates. Tests now work with FFmpeg builds that lack soxr or an MP3 encoder.
- 1.0.0 (2026-11-19): Initial release. Added the spectral cliff test, the lossy cliff scan, effective bandwidth, the bit-padding test (integer and float), the CLI with JSON/CSV/SVG output and parallel jobs, and the synthetic test suite.
