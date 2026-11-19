# Real-Hi-Res

Real-Hi-Res is a desktop app and command-line tool that scans your music library and tells you which files are genuine high-resolution audio and which ones are fakes. A "fake" hi-res file is one whose container advertises something like 24-bit/96 kHz or 24-bit/192 kHz while the audio inside actually came from a CD-quality master, a 48 kHz master, a lossy MP3/AAC file, or a 16-bit master that was padded with zeros or only given a volume change or dither on its way to 24 bits. The tool does not trust tags or file headers. It decodes the audio and checks the sample data itself against what the claimed resolution should contain.

![Real-Hi-Res desktop app listing fake and suspicious files, with the reasons and spectrum of the selected file](docs/gui-screenshot.png)

## Download and run (no installation needed)

Ready-to-run packages are attached to every release on the [Releases page](https://github.com/SU2H1/Real-Hi-Res/releases/latest). Python, NumPy and FFmpeg are bundled inside, so you only download, unpack and start it.

| System | Download | Start the app | Command line |
| --- | --- | --- | --- |
| Windows 10/11, 64-bit | `RealHiRes-windows-x64.zip` | `RealHiRes\RealHiRes.exe` | `RealHiRes\realhires-cli.exe "D:\Music"` |
| macOS 12 Monterey or later, Apple Silicon (M1 and later) | `RealHiRes-macos-arm64.dmg` (or `.zip`) | `RealHiRes.app` | `/Applications/RealHiRes.app/Contents/MacOS/realhires-cli ~/Music` |
| macOS 10.15 Catalina or later, Intel | `RealHiRes-macos-x86_64.dmg` (or `.zip`) | `RealHiRes.app` | `/Applications/RealHiRes.app/Contents/MacOS/realhires-cli ~/Music` |
| Linux, x86-64 (PC) | `RealHiRes-linux-x64.tar.gz` | `RealHiRes/RealHiRes` | `RealHiRes/realhires-cli ~/Music` |
| Linux, ARM64 (Raspberry Pi 4/5 with 64-bit OS, ARM laptops and servers) | `RealHiRes-linux-arm64.tar.gz` | `RealHiRes/RealHiRes` | `RealHiRes/realhires-cli ~/Music` |

If you are not sure which Mac you have, open the Apple menu and choose **About This Mac**. "Chip: Apple M…" means Apple Silicon, and "Processor: Intel…" means Intel. On Linux, `uname -m` prints `x86_64` for the x64 package or `aarch64` for the ARM64 package.

### macOS

Open the `.dmg` and drag **RealHiRes** into **Applications**. The app is not signed with an Apple Developer ID, so the first launch is blocked with a message that the app "is damaged" or "cannot be opened". On macOS 15 Sequoia and later, open **System Settings → Privacy & Security**, scroll down, and click **Open Anyway** next to the RealHiRes message. On older versions, right-click the app and choose **Open**. Alternatively, this Terminal command works on every version:

```bash
xattr -dr com.apple.quarantine /Applications/RealHiRes.app
```

### Linux

The Linux packages are built on glibc 2.28. They therefore run on practically every distribution from 2018 onwards, including Ubuntu 18.10+, Debian 10+, Linux Mint 20+, Fedora 29+, RHEL/Rocky/AlmaLinux 8+, openSUSE Leap 15.3+, Arch and Manjaro. The app needs a graphical desktop (X11, or Wayland with XWayland, which every mainstream desktop provides). The command-line tool runs anywhere, including on servers. Unpack the archive wherever you like to keep the app:

```bash
tar -xzf RealHiRes-linux-x64.tar.gz
```

Then optionally add Real-Hi-Res to your application menu and put `realhires-cli` on your `PATH` (in `~/.local/bin`). This installs for your user only and needs no root:

```bash
RealHiRes/install.sh
```

The app keeps running from the unpacked folder, so run `install.sh` again if you move the folder, and run `RealHiRes/uninstall.sh` to remove the menu entry.

### Windows

Unzip the archive and double-click `RealHiRes.exe`. The app is not code-signed, so SmartScreen may show "Windows protected your PC" the first time. Continue with **More info → Run anyway**.

## Desktop app (GUI)

Click **Add Folder…** (or **Add Files…**) and then **Scan**. Files are analysed in parallel, and results appear while the scan is running. The list shows only files judged **Fake hi-res** and **Suspicious** by default, so you can scroll straight through the problem files. Tick the other boxes (Real hi-res, Not hi-res, Lossy format, Error) to see everything, and use the search box to filter by file or folder name. Each checkbox shows how many files have that verdict.

Click a column header to sort by it. Click a row to see why that file got its verdict, its measurements, and its averaged spectrum, where the cliff of an upsampled file is easy to see. Double-click a row (or use **Show in Folder**) to open its folder, right-click it to copy its path, and use **Export CSV…** or **Export JSON…** to save the files currently listed. **Quick scan** analyses only the first 90 seconds of each file, which is faster on large libraries.

## Running from source

You need Python 3.9 or newer, NumPy, and FFmpeg (both `ffmpeg` and `ffprobe` must be on your `PATH`). FFmpeg does the decoding, so every format it understands works: FLAC, WAV, AIFF, ALAC/M4A, APE, WavPack, DSF/DFF (DSD), TTA, MKA, and lossy formats such as MP3, AAC, Opus and Vorbis. The GUI also needs Tkinter, which is included with the python.org installers for Windows and macOS. On Linux, install it with `sudo apt install python3-tk` (Debian/Ubuntu) or `sudo dnf install python3-tkinter` (Fedora).

On macOS you can install FFmpeg with `brew install ffmpeg`. On Ubuntu or Debian use `sudo apt install ffmpeg`, and on Windows use `winget install ffmpeg`.

Clone the repository and install it with pip, which gives you the `realhires` command and the `realhires-gui` desktop app:

```bash
git clone https://github.com/SU2H1/Real-Hi-Res.git
```

```bash
cd Real-Hi-Res && pip install .
```

You can also run it without installing by calling `python -m realhires` (command line) or `python -m realhires.gui` (desktop app) from inside the repository folder.

## Command-line usage

Point the tool at one or more files or folders. Folders are scanned recursively. (In a release package the command is called `realhires-cli` instead of `realhires`.)

```bash
realhires ~/Music/HiRes
```

The output is one line per file, and fake or suspicious files get their reasons printed underneath:

```
VERDICT       CLAIMED    REAL(EST)   BANDWIDTH  FILE
FAKE HI-RES   24/96      24/44.1     22.0k      fake_up_44_to_96.flac
              - Spectrum collapses by 114 dB around 22.05 kHz (level is down 6 dB at 21.1 kHz and down 20 dB by 21.4 kHz), the Nyquist limit of a 44.1 kHz master: upsampled from 44.1 kHz (high confidence).
FAKE HI-RES   24/96      16/96       47.8k      fake_padded_16_in_24.flac
              - Only 16 of 24 bits are used: the low 8 bits are always zero (16-bit audio padded into a 24-bit container).
FAKE HI-RES   24/44.1    16/44.1     21.9k      fake_gain_16_to_24.flac
              - Every sample lies on an even grid with a step of 181.2 levels of the 24-bit scale instead of 1. That is what a 16-bit master gives after a gain change of -3.0 dB (with or without dither) on its way into a 24-bit file, so the low bits hold no real detail and the resolution is still 16 bits.
FAKE HI-RES   24/96      lossy~17k   21.1k      fake_from_mp3.flac
              - Brick-wall low-pass at 17.0 kHz (87 dB drop within ~1 kHz, never recovers): typical of an MP3/AAC/Vorbis encoder, so this was made from a lossy file.
              - Spectrum collapses by 23 dB around 22.05 kHz (level is down 20 dB by 17.6 kHz), the Nyquist limit of a 44.1 kHz master: upsampled from 44.1 kHz (medium confidence).
FAKE HI-RES   24/192     24/96       47.9k      fake_up_96_to_192.wav
              - Spectrum collapses by 115 dB around 48.00 kHz ... upsampled from 96 kHz (high confidence). Still hi-res, but not the advertised 192 kHz.
SUSPICIOUS    24/96      24/96       16.4k      dull_96.wav
              - No content above 16.4 kHz although the format allows 48 kHz, and there is no brick-wall edge. The ultrasonic band holds only rounding noise, without the analogue noise floor that a converter running at this rate would leave, ...
REAL HI-RES   24/96      24/96       13.4k      analog_transfer_96.wav
REAL HI-RES   24/96      24/96       47.8k      real_24_96.flac
REAL HI-RES   24/192     24/192      95.5k      real_24_192.wav
NOT HI-RES    16/44.1    16/44.1     21.9k      cd_16_44.wav
LOSSY FORMAT  mp3/44.1   -           17.4k      src.mp3

Scanned 11 file(s): 5 fake hi-res, 1 suspicious, 3 real hi-res, 1 not hi-res, 1 lossy format
```

`CLAIMED` is the resolution the file advertises (bits/kHz, with `f` marking floating point). `REAL(EST)` is the resolution the audio actually carries according to the analysis. `BANDWIDTH` is the highest frequency at which the file contains real content. The `analog_transfer_96.wav` line shows a dull recording with nothing above 13.4 kHz that is still judged real, because its converter noise floor proves that it was digitised at 96 kHz (see "How detection works" below). With `-v`, that explanation and all raw measurements are printed for every file.

### Options

| Option | Meaning |
| --- | --- |
| `-v`, `--verbose` | Show the reasoning and raw measurements for every file, not just the fakes. |
| `--only fake` | List only files with a given verdict. You can repeat it, and the choices are `fake`, `suspicious`, `real`, `not-hires`, `lossy` and `error`. |
| `--json FILE` | Write every measurement to a JSON file. |
| `--csv FILE` | Write a CSV summary, which is handy for spreadsheets. |
| `--plot DIR` | Write an SVG spectrum plot for each file so you can check the verdict visually. |
| `-j N`, `--jobs N` | Analyse N files in parallel. The default is up to 4. |
| `-t SECONDS`, `--max-seconds SECONDS` | Analyse only the first N seconds of each file, which is faster but slightly less reliable. |
| `-n`, `--no-recursive` | Do not descend into sub-folders. |
| `--no-color` | Disable coloured output. Setting the `NO_COLOR` environment variable does the same. |

For example, the following command lists only the fakes in a library and saves plots and a CSV report:

```bash
realhires ~/Music --only fake --plot realhires-report/plots --csv realhires-report/report.csv
```

## Verdicts

| Verdict | Meaning |
| --- | --- |
| **REAL HI-RES** | The file claims hi-res and passed both the spectral test and the bit-depth test. A high-rate file with no music above 20 kHz is also real when its converter noise floor shows that it was digitised at that rate. |
| **FAKE HI-RES** | The file claims hi-res but was upsampled from a lower sample rate, made from a lossy file, or is a 16-bit master in a 24-bit or float container, either padded with zeros or only given a gain change or dither. The reasons tell you which. |
| **SUSPICIOUS** | The file claims a high sample rate but has no content above roughly 24.5 kHz (28.8 kHz when only rounding noise lies above it), no brick-wall edge, and no converter noise floor that would prove the rate. This can be a dull recording that was digitally filtered or produced, or a gentle-filter upsample. A 20-bit master in a 24-bit file is also reported as suspicious. Check the plot to decide. |
| **NOT HI-RES** | The file does not claim to be hi-res (16-bit at 44.1 or 48 kHz). If it was made from a lossy source, the reasons say so. |
| **LOSSY FORMAT** | The file is MP3, AAC, Opus, Vorbis or a similar format, which can never be real hi-res whatever its sample rate. |
| **ERROR** | The file could not be decoded. |

## How detection works

The tool runs two independent tests on the decoded samples. A file is judged real only when both pass.

### 1. Spectral test (sample rate)

Every digital master has a hard upper frequency limit at half its sample rate, called the Nyquist frequency. That limit is 22.05 kHz for 44.1 kHz, 24 kHz for 48 kHz, and 48 kHz for 96 kHz. Before audio is stored at a given rate it passes through a very steep anti-aliasing filter just below that limit. When someone later upsamples a 44.1 kHz master to 96 kHz, no new information is created, so the spectrum still falls off a cliff at about 22 kHz and contains nothing but the noise of the resampler above it. A real 96 kHz recording instead continues smoothly past 22 kHz, with cymbal and string harmonics, room tone and microphone noise, until it reaches its own limit near 48 kHz.

The tool averages the power spectrum of the whole track with an FFT of roughly 6 Hz resolution, using a Blackman-Harris window that has enough dynamic range to see a 24-bit noise floor. It then performs four measurements on that spectrum.

The first is the **cliff test**. For every common lower source rate (44.1, 48, 88.2, 96, 176.4 and 192 kHz) that is clearly below the file's rate, the tool compares the level just below that rate's Nyquist frequency (88–96 %) with the level just above it (102–112 %). A drop of 25 dB or more marks the file as upsampled from that rate, and a drop of 40 dB or more gives high confidence. Some resamplers, including FFmpeg's default one, have a soft transition that is still falling inside that upper window, so the tool also measures a **deep cliff** from the same lower band to the quietest quarter of 104–130 % of the Nyquist frequency, which such resamplers have reached by then. A deep drop of 45 dB or more counts as well, and 60 dB or more gives high confidence. Real music changes by only a few dB across that boundary. Even a natural roll-off of 72 dB per octave, far steeper than any microphone or instrument, stays below 20 dB on the narrow test and below 35 dB on the deep test.

When neighbouring rates such as 44.1 and 48 kHz both show a cliff, which happens when a resampler's transition band straddles both, the tool finds the frequency where the level has fallen 6 dB below the music's own trend (a straight line fitted to the spectrum just below the transition). Common resamplers put that point at 0.95–1.0 times the source Nyquist frequency. Steep ones such as SoX/soxr put it just below that frequency, and soft ones such as FFmpeg's swr put it exactly on it, so the tool picks the rate whose Nyquist frequency fits best. Because this test looks at a narrow window around each candidate edge, it still works when noise-shaped dither (for example in a fake DSD file) fills the top of the spectrum.

The second is the **lossy cliff scan**. MP3, AAC and Vorbis encoders throw away everything above about 16–20 kHz with an extremely steep low-pass. The tool searches the range from 11 to 20.3 kHz for a drop of 25 dB or more within about one kilohertz that never recovers at higher frequencies, which is the signature of a file that was decoded from a lossy source.

The third is the **effective bandwidth** measurement, which is the highest frequency where a continuous band of at least 300 Hz sits clearly above the noise floor. The floor reference is the top of the spectrum, capped near the theoretical quantisation noise of the file's bit depth so that content reaching Nyquist still counts. Content more than 110 dB below the spectral peak is ignored, and so is anything above an empty stretch wider than 3 kHz, because real spectra are continuous while the images and spurs that a weak resampler leaves far above its cliff are not. For an upsampled file the bandwidth is capped at the source's Nyquist frequency.

The fourth is the **converter noise floor** test. Every analogue-to-digital converter adds its own thermal noise, typically at −110 to −125 dBFS, which is 25–40 dB above the rounding noise of a 24-bit file and flat across the whole band up to the converter's own filter near the file's Nyquist frequency. An upsampled file holds only rounding noise above 22–24 kHz, because the lower-rate master never contained anything there. If a high-rate file has no content above about 24.5 kHz and no brick-wall edge, the tool therefore looks at the 26–40 kHz band. If that band is at least 15 dB above the rounding noise, flat within 6 dB, and shows no step at 22.05 or 24 kHz (a narrow drop of at most 8 dB and a deep drop of at most 15 dB), the file is real: it is typically an old or dull analogue recording transferred at a high rate. If the band holds only rounding noise, nothing proves the rate either way and the file is suspicious. Content that stops between 24.5 and 28.8 kHz is treated the same way, because a soft resampler can smear the 24 kHz edge of a 48 kHz master up to there.

### 2. Bit-depth test

FFmpeg decodes every integer format into left-justified 32-bit samples, so a 24-bit sample keeps its 8 unused bits at zero and a 16-bit sample keeps 16 zero bits. For every non-zero sample the tool records the lowest bit that is set. In real 24-bit audio the 24th bit is set in about half of all samples, because the lowest bits carry noise and fine detail. If a 16-bit master was simply padded to 24 bits, nothing below bit 16 is ever set, and the file is reported as "16 of 24 bits used". A threshold of 0.5 % of samples means that a handful of clipped or odd samples cannot hide the padding. The same test catches 16-bit audio stored as 32-bit float, because every sample is then an exact 16-bit value.

A 16-bit master that is given a volume change or dither on its way to 24 bits fills the low bits, so the test above sees 24 bits in use. Its samples still sit on a regular **value grid**, though. A gain g turns each 16-bit step into 256 × g steps of the 24-bit scale, and dither only blurs each grid point by a few steps. The tool therefore keeps a histogram of the sample values near zero (within ±65,536 on the 24-bit scale, separately for each channel) and looks for a periodic comb in it with a Fourier transform. Real 24-bit audio has a smooth histogram and a comb strength below 0.05, while a scaled or dithered 16-bit master scores 0.85–1.0, and a grid with a strength of 0.35 or more counts. The grid step gives the gain that was applied, and together with the file's peak level it gives the bit depth of the master, because a b-bit master can never span more than 2^b grid steps. A 16-bit grid is FAKE and a 17–22-bit grid is SUSPICIOUS. The test works the same in float files. It also catches many FFmpeg upsamples of 16-bit masters by a factor of two, which keep every second sample unchanged, so about half of the samples remain on the grid.

## Limitations

No automatic tool can be certain, so please read the reasons and look at the plots for files that matter to you. The following cases are known to be hard.

The source sample rate is now chosen by where the level has fallen 6 dB, which places both steep and soft resamplers (including FFmpeg's default one) on the right rate in tests with music of ordinary brightness. For very dull music, whose spectrum is already falling by 30–40 dB per octave when it reaches the resampler's transition band, a soft resampler can still make a 44.1 kHz master look like a 48 kHz one. The verdict is FAKE either way, but the source rate is then approximate. Extremely soft resamplers (for example FFmpeg with `filter_size=16`) leave a cliff too shallow to prove an upsample from 48 kHz, and only the value-grid test catches such a file when its master was 16-bit.

A 16-bit master that was converted to 24-bit or float with a volume change or dither is now caught by the value-grid test. The grid disappears when the master was really processed on the way, for example by equalisation, compression, a fade over the whole track, or resampling to a non-integer ratio. If the sample rate was raised as well, the spectral test still catches the file, but a "16/44.1 to 24/44.1" conversion with real processing in between is not detected.

Old analogue recordings often have no content above 20 kHz even when they were transferred at 96 kHz or higher. When the converter's noise floor is visible in the 26–40 kHz band, they are now judged REAL. When the ultrasonic band holds only rounding noise, because the file was produced digitally, low-passed or heavily noise-reduced at the high rate, they remain SUSPICIOUS, because the data cannot prove which one they are. A fake that was re-recorded through a DAC and an ADC at the high rate carries a genuine converter noise floor. It is still reported as SUSPICIOUS rather than REAL as long as the master's own noise floor makes a step at 22.05 or 24 kHz.

A real hi-res file whose ultrasonic band contains only analogue tape or microphone hiss is reported as REAL, because that hiss is genuinely captured above 22 kHz.

MP3 files encoded at 320 kbps low-pass at about 20.5 kHz, which overlaps with ordinary CD anti-aliasing filters, so a lossy source is not always detectable in a 44.1 or 48 kHz file. Upsampled to hi-res, such a file is still caught by the cliff test.

## Development

Run the test suite with pytest. The tests generate synthetic real and fake files with FFmpeg and NumPy, so no audio files are stored in the repository. They cover genuine 24/48, 24/96 and 24/192 files, upsamples from 44.1→96, 48→96 (with soxr and with FFmpeg's default resampler), 44.1→192 and 96→192, bit-padded 16-in-24 and 16-in-float files, 16-bit masters given a gain change or dither into 24-bit and float, MP3-sourced files, a dull 96 kHz analogue transfer with converter noise, a dull 48 kHz transfer upsampled to 96 kHz, and dull and silent files.

```bash
python3 -m pytest -q
```

### Building the packages and publishing a release

The release packages are built by GitHub Actions ([`.github/workflows/build.yml`](.github/workflows/build.yml)) in six jobs: Windows x64, macOS Apple Silicon, macOS Intel, Linux x64 and Linux ARM64. The Linux jobs run inside an AlmaLinux 8 container so that the packages only need glibc 2.28. Every job runs [`packaging/ci_build.sh`](packaging/ci_build.sh), which goes through the following steps in order:

1. It downloads a static FFmpeg.
2. It runs the test suite with that FFmpeg.
3. It builds the package with PyInstaller.
4. It runs [`packaging/check_compat.py`](packaging/check_compat.py), which fails the build if any bundled file needs a newer glibc or macOS than the package claims, or (on macOS) lacks the right CPU architecture.
5. It smoke-tests the frozen command-line tool with an empty `PATH`.
6. It packages the result.

Every push uploads the packages as workflow artifacts. A GitHub Release is published automatically in two cases. The first is when `main` receives a commit whose version (`__version__` in `realhires/__init__.py`, kept equal to `version` in `pyproject.toml`) has not been released yet, so bumping the version and merging into `main` releases it. The second is when a tag matching that version is pushed:

```bash
git tag v1.1.0
```

```bash
git push origin v1.1.0
```

To build a package on your own machine, install the build tools and run the same script with the name of the archive you want. The result appears in `out/`.

```bash
pip install numpy pyinstaller pillow pytest
```

```bash
ASSET=RealHiRes-local bash packaging/ci_build.sh
```

On Linux, run it inside AlmaLinux 8 for the widest compatibility (`bash packaging/linux/setup_almalinux8.sh` prepares such a container, after which you set `PYTHON=/opt/venv/bin/python`). A build made directly on a newer distribution only runs on that distribution's glibc or newer, and `check_compat.py` will say so.

The project layout is documented in [CLAUDE.md](CLAUDE.md).

## License

No license has been chosen yet.
