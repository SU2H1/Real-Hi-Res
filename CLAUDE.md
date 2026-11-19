# CLAUDE.md

This file guides Claude (and other contributors) working in this repository.

## Standing instruction: keep the docs current

Every change to behaviour, options, thresholds, verdicts, file layout, packaging or dependencies must update **both** `README.md` (the user-facing documentation) and this `CLAUDE.md` (the contributor/agent notes) in the same commit. If you add a CLI flag, document it in the README options table. If you change a detection threshold, update the "How detection works" section of the README and the tunables table below. If you change the GUI, check that the README "Desktop app" section and `docs/gui-screenshot.png` still match it. If you change packaging, update the README "Download" and "Building the packages" sections and `packaging/README-RELEASE.txt`. Add a line to the changelog at the bottom of this file for each notable change, and bump `__version__` in `realhires/__init__.py` and `version` in `pyproject.toml` together.

## What the project does

`realhires` scans audio files or folders and classifies each file as REAL HI-RES, FAKE HI-RES, SUSPICIOUS, NOT HI-RES, LOSSY FORMAT or ERROR. It decodes the audio with FFmpeg and inspects the samples, so it never trusts tags or headers. There is a CLI (`realhires`) and a Tkinter desktop app (`realhires-gui`). Both are also shipped as self-contained PyInstaller packages, with FFmpeg bundled, for Windows x64, macOS arm64 (12+), macOS x86_64 (10.15+), Linux x64 and Linux arm64 (glibc 2.28+).

## Commands

| Task | Command |
| --- | --- |
| Run the test suite (about 10 s, it synthesises its own audio with ffmpeg) | `python3 -m pytest -q` |
| Run the CLI from the source tree | `python3 -m realhires PATH... [-v] [--json F] [--csv F] [--plot DIR]` |
| Run the GUI from the source tree (needs Tkinter) | `python3 -m realhires.gui [PATH...]`. Passing paths starts a scan automatically. |
| Install the `realhires` and `realhires-gui` commands | `pip install .` |
| Build, verify and package locally (what CI runs) | `ASSET=RealHiRes-local bash packaging/ci_build.sh`. The output goes to `out/`. Linux builds belong inside AlmaLinux 8 (`packaging/linux/setup_almalinux8.sh`, then `PYTHON=/opt/venv/bin/python`). |
| Check binary compatibility of a build | `python packaging/check_compat.py dist/RealHiRes --max-glibc 2.28`, or `... dist/RealHiRes.app --max-macos 12.0 --arch arm64` |
| Smoke-test a built package | `python packaging/smoke_test.py dist/RealHiRes/realhires-cli` |
| Regenerate the app icon | `python packaging/make_icon.py`, which writes `realhires/assets/icon.png` |
| Publish a release | Bump `__version__` and `pyproject.toml` `version` together, add a changelog line, and get the commit onto `main`. The `release` job then tags `vX.Y.Z` and publishes the five packages. Pushing a matching tag `vX.Y.Z` works too, and a tag that does not match `__version__` fails the job. |

The runtime requirements are Python 3.9 or newer, NumPy, and `ffmpeg`/`ffprobe` (bundled or on `PATH`). The GUI additionally needs Tkinter (standard library). There are no other runtime dependencies, and plots are hand-written SVG or Tk canvas, so matplotlib is not needed. Keep it that way unless there is a strong reason.

## Layout

| File | Role |
| --- | --- |
| `realhires/probe.py` | Contains `find_tool()`/`tool()` (FFmpeg lookup order: `$REALHIRES_FFMPEG_DIR`, then the PyInstaller bundle `_MEIPASS/ffmpeg`, then `ffmpeg/` next to the executable, then `PATH`), `probe()` (ffprobe to `AudioInfo`), and `iter_pcm()`, which streams decoded audio as left-justified int32 `(frames, channels)` blocks. `LOSSY_CODECS` lives here. `_no_window()` suppresses console windows on Windows. |
| `realhires/analysis.py` | Contains `SpectrumAccumulator` (streaming Welch spectrum, Blackman-Harris window, about 6 Hz bins), `BitDepthAccumulator` (lowest-set-bit histogram), `GridAccumulator` with `_grid_step()` and the `Grid` result (per-channel histogram of 24-bit values within ±65,536, whose DFT reveals the comb left by a scaled or dithered 16-bit master), `analyze_spectrum()` (gap-limited bandwidth, narrow and deep cliff tests, the −6 dB tie-break via `_midpoint_frequency()`, the lossy cliff scan and the 26–40 kHz converter floor), `converter_noise_floor()` (whether that floor proves a high-rate capture), and `compact_spectrum()` (a 900-point max-hold spectrum for the GUI). All detection tunables are module constants. |
| `realhires/verdict.py` | Contains `analyze_file()` (the per-file pipeline, which never raises and turns failures into ERROR; `keep_spectrum` is for SVG and `keep_plot` is for the GUI; the grid test only runs for files that claim more than 16 bits or float), `_judge()` (measurements to verdict plus reasons), and `FileResult` (with `to_dict()` for JSON/CSV). `effective_bits` is the lower of the lowest-set-bit result and the grid's bit depth. |
| `realhires/report.py` | Terminal table, summary, JSON/CSV writers, and the dependency-free SVG spectrum plot. |
| `realhires/cli.py` | Handles argparse, file collection (`AUDIO_EXTENSIONS`, `collect()`), the `ProcessPoolExecutor` fan-out, and ordered streaming output. |
| `realhires/gui.py` | The Tkinter app. The `Scanner` thread uses a spawn-context `ProcessPoolExecutor` and a `queue` that `App._poll()` drains every 100 ms. Its pieces are a `ttk.Treeview` result list (default filter: Fake plus Suspicious), filter checkboxes with counts, search, sortable columns, a details text panel, a `SpectrumCanvas`, and export/reveal/copy actions. |
| `packaging/realhires.spec` | PyInstaller spec. It builds one folder `RealHiRes/` holding the `RealHiRes` GUI (windowed) and the `realhires-cli` console executable, which share one runtime, with FFmpeg in `_internal/ffmpeg/`. On macOS it is wrapped as `RealHiRes.app`, with the CLI at `Contents/MacOS/realhires-cli`. |
| `packaging/entry_gui.py`, `packaging/entry_cli.py` | Frozen entry points. Both call `multiprocessing.freeze_support()`. |
| `packaging/ci_build.sh` | The single build pipeline used by every CI job. It fetches FFmpeg, runs pytest with it, runs PyInstaller, runs `check_compat.py` (plus `codesign --verify` on macOS), smoke-tests, and then packages: zip on Windows, `.dmg` plus `.zip` on macOS, `.tar.gz` with `install.sh`/`uninstall.sh`/icon on Linux. |
| `packaging/check_compat.py` | Fails if any ELF file needs a GLIBC symbol version above `--max-glibc` (`objdump -T`), or any Mach-O file has a `minos` above `--max-macos` or lacks the `--arch` slice (`otool -l`, `lipo -archs`). |
| `packaging/linux/setup_almalinux8.sh` | Prepares the AlmaLinux 8 build container: dnf python3.12 with tkinter and the shared libpython, binutils, and a venv in `/opt/venv`. |
| `packaging/linux/install.sh`, `uninstall.sh` | Shipped in the Linux tarball. They add or remove `~/.local/share/applications/realhires.desktop`, the hicolor icon, and a `~/.local/bin/realhires-cli` symlink. They need no root. |
| `packaging/make_icon.py`, `realhires/assets/icon.png` | The NumPy-drawn app icon. It is used as the Tk window icon, converted by PyInstaller (with Pillow) into the `.ico`/`.icns`, and used as the Linux menu icon. |
| `packaging/fetch_ffmpeg.py` | Downloads static ffmpeg and ffprobe into `build/ffmpeg`: BtbN LGPL builds for Windows x64 and Linux x64/arm64, Martin Riedl builds for macOS arm64 (needs macOS 12), evermeet.cx builds for macOS x86_64. It also writes `FFMPEG-NOTICE.txt`. |
| `packaging/smoke_test.py` | Runs a frozen CLI with an empty `PATH` on three generated WAVs and checks the verdicts. |
| `packaging/README-RELEASE.txt`, `packaging/RELEASE_NOTES.md` | The README copied into each package, and the body of the GitHub Release. |
| `.github/workflows/build.yml` | On every push or PR, it runs the jobs `windows` (windows-latest), `macos` (arm64 on macos-latest with min 12.0 (Riedl FFmpeg), x86_64 on macos-15-intel with min 10.15 (evermeet FFmpeg), each with NumPy wheels downloaded for that minimum via `pip download --platform`) and `linux` (x64 on ubuntu-24.04 and arm64 on ubuntu-24.04-arm, both in an `almalinux:8` container). All of them call `packaging/ci_build.sh` and upload `out/*`. The `release` job runs on `v*` tags, and on pushes to `main` whose `__version__` has no tag yet. It publishes everything with softprops/action-gh-release, which also creates the tag. Cloud sessions cannot push tags themselves, because the git proxy only accepts their own branch, so the main-push path is how they release. |
| `tests/conftest.py` | `music_like()` synthesises pink-ish stereo noise, and `converter()` adds what an ADC running at a given rate leaves behind (white noise at −115 dBFS and a half-band anti-alias filter). The session fixture `library` builds real and fake files with ffmpeg. `hq_resample()` falls back from soxr to a long swr filter, and MP3 fixtures are omitted (and their tests skipped) when the FFmpeg build has no MP3 encoder. |
| `tests/test_detection.py` | End-to-end verdict tests (including soft-resampler, gain-changed, dithered and analogue-transfer cases), a `GridAccumulator` unit test, and a CLI smoke test with all exports. |
| `docs/gui-screenshot.png` | The screenshot shown in the README. |

## Detection tunables (`realhires/analysis.py`)

| Constant | Value | Meaning |
| --- | --- | --- |
| `CLIFF_DB` / `CLIFF_STRONG_DB` | 25 / 40 dB | The narrow cliff: the level drop from 88–96 % to 102–112 % of a candidate source Nyquist that means "upsampled", at medium and high confidence. |
| `DEEP_CLIFF_DB` / `DEEP_CLIFF_STRONG_DB` | 45 / 60 dB | The deep cliff: the drop from 88–96 % to the 25th percentile of 104–130 % of that Nyquist, which catches soft resamplers (FFmpeg's default swr) that are still falling inside the narrow window. Natural roll-offs of up to 72 dB/octave stay below 35 dB. |
| `MID_RATIO` | 0.975 | When several candidates pass (and a neighbour within ±10 % reaches half the threshold), pick the one whose Nyquist best matches the −6 dB point (soxr about 0.955, swr 1.0). |
| `MAX_TREND_DB_PER_OCT` | 45 dB/oct | The steepest music slope that `_midpoint_frequency()` extrapolates. Its trend line is fitted over 68–80 % of the lowest candidate Nyquist and anchored at the top of that band. |
| `STEEP_DROP_DB` | 20 dB | The drop across the bandwidth edge that counts as a brick wall, used by the fallback upsample rule. |
| `LOSSY_CLIFF_DB`, `LOSSY_MAX_HZ` | 25 dB, 20.3 kHz | The lossy-encoder low-pass search, run over 11–20.3 kHz. |
| `CONTENT_MARGIN_DB` | 10 dB | How far content must sit above the floor reference. |
| `RELEVANT_RANGE_DB` | 110 dB | Content further below the spectral peak than this is ignored, which drops resampler images. |
| `BANDWIDTH_GAP_HZ` | 3 kHz | Content above an empty stretch wider than this does not extend the bandwidth (it drops swr images and spurs). For an upsampled file the bandwidth is also capped at the source Nyquist. |
| `ULTRASONIC_HZ` | 24.5 kHz | A high-rate file with bandwidth below this and no cliff is SUSPICIOUS, unless `converter_noise_floor()` holds. |
| `SMEAR_HZ` | 28.8 kHz | A bandwidth between `ULTRASONIC_HZ` and this, with only rounding noise in 26–40 kHz, is also SUSPICIOUS (a soft resampler smears the 24 kHz edge this far). |
| `ANALOG_FLOOR_DB`, `FLOOR_TILT_DB` | 15 dB, 6 dB | `converter_noise_floor()`: the 26–40 kHz median must be this far above the rounding noise (TPDF dither adds only about 5 dB, real converters 25–40 dB), and its two halves must agree within this. |
| `FLOOR_STEP_DB`, `FLOOR_DEEP_STEP_DB` | 8 dB, 15 dB | ... and the narrow and deep cliffs at 22.05 and 24 kHz must stay below these (a re-recorded CD shows its own 16-bit floor stepping down). |
| `GRID_WINDOW`, `GRID_MIN_SAMPLES` | ±65,536, 5,000 | The value range (24-bit scale) of the grid histogram, and the samples per channel needed to decide. |
| `GRID_MIN_STEP`, `GRID_MAX_STEP` | 6, 2,048 | The grid steps searched for (a 16-bit master at −32 to +18 dB). |
| `GRID_STRENGTH`, `GRID_FULL` | 0.35, 0.8 | The normalised comb strength that proves a grid (real 24-bit audio stays below 0.05), and the strength above which the reason says that every sample is on it (below it, it describes a partial grid such as a 2× upsample leaves). |
| `BitDepthAccumulator.effective_bits(threshold=0.005)` | 0.5 % | The share of samples needed for a bit position to count as used. |

Candidate source rates are limited to `rate * 1.5 <= file_rate`, so a 96 kHz file is never accused of being upsampled from 88.2 kHz. The grid's bit depth is `16` when `log2(2 * peak / step)` is at most 16.1, and the next whole number above it otherwise; grids of 23 bits or more are ignored.

## Conventions and gotchas

- ffmpeg output is always `-f s32le -acodec pcm_s32le`. The bit-depth and value-grid tests rely on ffmpeg left-justifying integer samples, and on float-to-s32 conversion being undithered, which is the swresample default. The grid test shifts samples right by 8, so it always works on the 24-bit scale.
- Always call FFmpeg through `probe.tool("ffmpeg")` / `tool("ffprobe")` with `**_no_window()`, never through a bare `"ffmpeg"` string, or the packaged builds break.
- Spectrum levels are per bin and normalised so that a full-scale sine reads 0 dB. They are therefore much lower (about -60 dB for music, about -180 dB for a 24-bit floor at a 16k FFT) than the levels shown by spectrum viewers with small FFTs. `quantization_floor_db()` gives the theoretical floor.
- `analyze_file()` must never raise. Any exception becomes `verdict=ERROR` with the message in `reasons`.
- `analyze_spectrum()` receives the lower of the lowest-set-bit depth and the grid depth, so a file found to be a scaled 16-bit master gets a 16-bit quantisation floor for its bandwidth and converter-floor measurements.
- When you tune the spectral thresholds, test against both genuine spectra (steep natural roll-offs, with and without a converter noise floor) and fakes made with soxr and with FFmpeg's default swr at several filter sizes. Synthetic music without any noise floor is unrealistic; real converters always add one.
- `FileResult.spectrum` holds large arrays. It is only kept for `--plot` and is cleared after plotting. `FileResult.plot` is the small GUI version. Anything that the estimated-resolution logic needs must be copied onto `FileResult` fields, never read from `spectrum`.
- GUI worker processes use the **spawn** context, so any script that starts the GUI must guard its code with `if __name__ == "__main__":`. Otherwise every worker re-runs the script and opens another window. Frozen entry points must call `multiprocessing.freeze_support()` first.
- Every external program must be started with `probe.external_env()` (it is included in `_no_window()`). A frozen Linux build sets `LD_LIBRARY_PATH` to its bundled libraries, and a system ffmpeg or `xdg-open` that inherits it loads the bundle's old `libstdc++`/`libX11` and crashes.
- Linux packages must stay buildable on glibc 2.28, because CI enforces it. Do not add `MAX_GLIBC` exceptions. Instead, find out why a library needs a newer glibc. The spec deliberately leaves out `libfontconfig`/`libfreetype` so that the desktop's own copies are used, because an old bundled fontconfig cannot parse newer configs. Frozen Tk also needs the system `libxcb`, which every X11/XWayland desktop has.
- macOS minimums are declared through `REALHIRES_MACOS_MIN` (into `LSMinimumSystemVersion`) and verified by `check_compat.py`. If a dependency raises its minimum, CI fails, and you must then either raise the documented minimum (README, RELEASE_NOTES, README-RELEASE, the workflow) or pin the dependency.
- Do not edit `Info.plist` or any file inside the built `.app` after PyInstaller has signed it, because that breaks the ad-hoc signature and Apple Silicon then reports the app as "damaged".
- Executable names must not differ only by case (`RealHiRes` vs `realhires`), because Windows and macOS file systems are case-insensitive. That is why the CLI in packages is `realhires-cli`.
- On Linux the GUI uses the `clam` ttk theme, whose progress bar is invisible by default, so `main()` sets an explicit bar colour.
- You can test the GUI headlessly with `xvfb-run` and take screenshots with ImageMagick `import -window root out.png`. Without a display, `gui.main()` prints a clear message and returns 1.
- Local Linux package builds: inside the cloud dev container, dockerd can be started and `almalinux:8` run with `--network host`, with the proxy CA mounted and the repo `baseurl` used instead of `mirrorlist`.
- The tests must stay self-contained. Generate audio in fixtures instead of committing audio files. They must pass with both the distro FFmpeg and the bundled BtbN/Riedl builds used in CI.
- Always use full, specific sentences in user-facing reasons, because they are printed verbatim in the CLI and the GUI.

## Changelog

- 1.2.0 (2026-11-19): Better accuracy on the three known-hard cases. The source rate is now chosen by the −6 dB point against the music's trend instead of the −20 dB point, so FFmpeg's default resampler no longer turns a 44.1 kHz master into "48 kHz". A deep cliff test (to the quietest quarter of 104–130 % of the Nyquist) catches soft-resampler upsamples that used to pass as REAL, including 48→96 kHz with FFmpeg defaults. The bandwidth now ignores content above a gap of more than 3 kHz and is capped at the source Nyquist for upsampled files. Added the value-grid test, which catches 16-bit masters converted to 24-bit or float with only a gain change or dither, and many 2× upsamples of 16-bit masters. Added the converter-noise-floor test, which proves that a dull or old analogue recording was digitised at its high rate (REAL instead of SUSPICIOUS), and a smear zone up to 28.8 kHz for soft-resampler edges. New `FileResult` fields: `upsample_mid_hz`, `deep_cliffs_db`, `converter_floor_db`, `grid_step`, `grid_bits` and `grid_gain_db`. The CSV gained `grid_step`. The verbose CLI and the GUI details show the new measurements. The GUI screenshot was regenerated.
- 1.1.0 (2026-11-19): Broadened Linux and macOS support. Linux packages are now built on AlmaLinux 8 (glibc 2.28) for x64 and arm64 and come with `install.sh`/`uninstall.sh`. A macOS Intel build (10.15+) was added next to arm64 (12+), as `.dmg` plus `.zip`, with an app icon. CI now checks compatibility of every bundled binary. Fixed `LD_LIBRARY_PATH` leaking from the frozen app into child processes, and added clear errors when Tk or a display is missing. Added the Tkinter desktop GUI, with a scrollable list of fake and suspicious files, filters, search, sorting, a details panel with a spectrum, and export. Added self-contained release packages (PyInstaller with bundled FFmpeg) for Windows x64, macOS arm64 and Linux x64, built and published by GitHub Actions, either on `v*` tags or automatically when `main` receives a version that has not been released yet. Added the bundled-FFmpeg lookup and suppressed console windows on Windows. Added the edge-frequency tie-break between neighbouring source rates. Tests now work with FFmpeg builds that lack soxr or an MP3 encoder.
- 1.0.0 (2026-11-19): Initial release. Added the spectral cliff test, the lossy cliff scan, effective bandwidth, the bit-padding test (integer and float), the CLI with JSON/CSV/SVG output and parallel jobs, and the synthetic test suite.
