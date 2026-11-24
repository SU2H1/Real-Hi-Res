# Real-Hi-Res in Rust

This folder holds the Rust port of Real-Hi-Res. It judges every file exactly as the Python version in `realhires/` does, and it shares that version's settings file and saved results. The user-facing description is in the main [README](../README.md#rust-version-preview); this file is for building and working on the code.

## Building and testing

You need Rust 1.82 or newer and FFmpeg (`ffmpeg` and `ffprobe` on the `PATH`, or in an `ffmpeg` folder next to the executable, or in the folder `$REALHIRES_FFMPEG_DIR` names). Build the optimised programs with:

```bash
cargo build --release
```

Run the unit tests with:

```bash
cargo test
```

The comparison with the Python version is a pytest in the main repository, because it reuses the synthetic test library of the Python tests. Run it from the repository root after the release build:

```bash
python3 -m pytest -q tests/test_rust_parity.py
```

## Layout

The workspace has one library and the programs built on it.

| Crate | Contents |
| --- | --- |
| `crates/core` (library `realhires`) | The detection pipeline and everything around it, one module per Python module: `probe` (ffprobe metadata, tags, the PCM stream from ffmpeg), `analysis` (Welch spectrum, bit-depth histogram, value grid, every spectral rule), `verdict` (`FileResult`, the judging rules, `analyze_file`), `settings`, `store` (the shared SQLite results), `fix`, `report` (table, JSON, CSV, SVG), `imaging` (spectrogram PNG), `eta`, `library` (album and artist groups) and `covers`. Three helper modules have no Python counterpart: `dsp` holds the NumPy functions the rules use, with NumPy's exact semantics (median, linear percentile, `convolve(mode="same")`, `polyfit`, window functions); `pyfmt` formats numbers exactly as Python does (`{:g}`, `{:%}`, `repr(float)`, `round()`), so every reason is the same text; `json` reads and writes Python's JSON dialect (`NaN`, `Infinity`, Python's float notation), which the saved results and the settings key depend on. `paths` matches `os.path.abspath` so both versions key saved results by the same path string. |
| `crates/cli` (program `realhires`) | The command-line tool, with the options of `realhires/cli.py`. Files are analysed by worker threads, and the results are printed and saved in file order by the main thread. |

## Keeping it equal to the Python version

Every change to the detection rules, the reasons and summaries, the settings, the saved-results format, the CLI options or Fix must be made here too, in the same commit, and `tests/test_rust_parity.py` must pass. The version in `Cargo.toml` (`[workspace.package] version`) is kept equal to `realhires.__version__`, which `test_rust_port_has_the_same_version` checks, because both versions write it into the saved results they share. `verdict::DETECTOR_VERSION` must equal the Python `DETECTOR_VERSION` for the same reason.
