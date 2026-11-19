# Real-Hi-Res

Real-Hi-Res is a command-line tool that scans your music library and tells you which files are genuine high-resolution audio and which ones are fakes. A "fake" hi-res file is one whose container advertises something like 24-bit/96 kHz or 24-bit/192 kHz while the audio inside actually came from a CD-quality master, a 48 kHz master, a lossy MP3/AAC file, or a 16-bit master padded with zeros. The tool does not trust tags or file headers. It decodes the audio and checks the sample data itself against what the claimed resolution should contain.

## Requirements

You need Python 3.9 or newer, NumPy, and FFmpeg (both `ffmpeg` and `ffprobe` must be on your `PATH`). FFmpeg does the decoding, so every format it understands works: FLAC, WAV, AIFF, ALAC/M4A, APE, WavPack, DSF/DFF (DSD), TTA, MKA, and lossy formats such as MP3, AAC, Opus and Vorbis.

On macOS you can install FFmpeg with `brew install ffmpeg`. On Ubuntu or Debian use `sudo apt install ffmpeg`, and on Windows use `winget install ffmpeg`.

## Installation

Clone the repository and install it with pip, which also gives you the `realhires` command:

```bash
git clone https://github.com/SU2H1/Real-Hi-Res.git
```

```bash
cd Real-Hi-Res && pip install .
```

You can also run it without installing by calling `python -m realhires` from inside the repository folder.

## Usage

Point the tool at one or more files or folders. Folders are scanned recursively.

```bash
realhires ~/Music/HiRes
```

The output is one line per file, and fake or suspicious files get their reasons printed underneath:

```
VERDICT       CLAIMED    REAL(EST)   BANDWIDTH  FILE
FAKE HI-RES   24/96      24/44.1     22.0k      fake_up_44_to_96.flac
              - Spectrum collapses by 113 dB around 22.05 kHz (level is down 20 dB by 21.4 kHz), the Nyquist limit of a 44.1 kHz master: upsampled from 44.1 kHz (high confidence).
FAKE HI-RES   24/96      16/96       47.8k      fake_padded_16_in_24.flac
              - Only 16 of 24 bits are used: the low 8 bits are always zero (16-bit audio padded into a 24-bit container).
FAKE HI-RES   24/96      lossy~17k   21.1k      fake_from_mp3.flac
              - Brick-wall low-pass at 17.0 kHz (87 dB drop within ~1 kHz, never recovers): typical of an MP3/AAC/Vorbis encoder, so this was made from a lossy file.
FAKE HI-RES   24/192     24/96       47.9k      fake_up_96_to_192.wav
              - Spectrum collapses by 114 dB around 48.00 kHz ... upsampled from 96 kHz (high confidence). Still hi-res, but not the advertised 192 kHz.
REAL HI-RES   24/96      24/96       47.8k      real_24_96.flac
REAL HI-RES   24/192     24/192      95.5k      real_24_192.wav
NOT HI-RES    16/44.1    16/44.1     21.9k      cd_16_44.wav
LOSSY FORMAT  mp3/44.1   -           17.4k      src.mp3

Scanned 8 file(s): 4 fake hi-res, 2 real hi-res, 1 not hi-res, 1 lossy format
```

`CLAIMED` is the resolution the file advertises (bits/kHz, with `f` marking floating point). `REAL(EST)` is the resolution the audio actually carries according to the analysis. `BANDWIDTH` is the highest frequency at which the file contains real content.

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
| **REAL HI-RES** | The file claims hi-res and passed both the spectral test and the bit-depth test. |
| **FAKE HI-RES** | The file claims hi-res but was upsampled from a lower sample rate, made from a lossy file, or is 16-bit audio padded into a 24-bit or float container. The reasons tell you which. |
| **SUSPICIOUS** | The file claims a high sample rate but has no content above roughly 24.5 kHz and no brick-wall edge. This can be an old or dull recording, a gentle-filter upsample, or a 20-bit master in a 24-bit file. Check the plot to decide. |
| **NOT HI-RES** | The file does not claim to be hi-res (16-bit at 44.1 or 48 kHz). If it was made from a lossy source, the reasons say so. |
| **LOSSY FORMAT** | The file is MP3, AAC, Opus, Vorbis or a similar format, which can never be real hi-res whatever its sample rate. |
| **ERROR** | The file could not be decoded. |

## How detection works

The tool runs two independent tests on the decoded samples. A file is judged real only when both pass.

### 1. Spectral test (sample rate)

Every digital master has a hard upper frequency limit at half its sample rate, called the Nyquist frequency. That limit is 22.05 kHz for 44.1 kHz, 24 kHz for 48 kHz, and 48 kHz for 96 kHz. Before audio is stored at a given rate it passes through a very steep anti-aliasing filter just below that limit. When someone later upsamples a 44.1 kHz master to 96 kHz, no new information is created, so the spectrum still falls off a cliff at about 22 kHz and contains nothing but the noise of the resampler above it. A real 96 kHz recording instead continues smoothly past 22 kHz, with cymbal and string harmonics, room tone and microphone noise, until it reaches its own limit near 48 kHz.

The tool averages the power spectrum of the whole track with an FFT of roughly 6 Hz resolution, using a Blackman-Harris window that has enough dynamic range to see a 24-bit noise floor. It then performs three measurements on that spectrum.

The first is the **cliff test**. For every common lower source rate (44.1, 48, 88.2, 96, 176.4 and 192 kHz) that is clearly below the file's rate, the tool compares the level just below that rate's Nyquist frequency (88–96 %) with the level just above it (102–112 %). A drop of 25 dB or more marks the file as upsampled from that rate. A drop of 40 dB or more gives high confidence. Real music changes by only a few dB across that boundary. Because this test looks at a narrow window around each candidate edge, it still works when noise-shaped dither (for example in a fake DSD file) fills the top of the spectrum.

The second is the **lossy cliff scan**. MP3, AAC and Vorbis encoders throw away everything above about 16–20 kHz with an extremely steep low-pass. The tool searches the range from 11 to 20.3 kHz for a drop of 25 dB or more within about one kilohertz that never recovers at higher frequencies, which is the signature of a file that was decoded from a lossy source.

The third is the **effective bandwidth** measurement, which is the highest frequency where a continuous band of at least 300 Hz sits clearly above the noise floor. The floor reference is the top of the spectrum, capped near the theoretical quantisation noise of the file's bit depth so that content reaching Nyquist still counts. Content more than 110 dB below the spectral peak is ignored. If a high-sample-rate file has no content above about 24.5 kHz but also no brick-wall edge, it is reported as suspicious rather than fake.

### 2. Bit-depth test

FFmpeg decodes every integer format into left-justified 32-bit samples, so a 24-bit sample keeps its 8 unused bits at zero and a 16-bit sample keeps 16 zero bits. For every non-zero sample the tool records the lowest bit that is set. In real 24-bit audio the 24th bit is set in about half of all samples, because the lowest bits carry noise and fine detail. If a 16-bit master was simply padded to 24 bits, nothing below bit 16 is ever set, and the file is reported as "16 of 24 bits used". A threshold of 0.5 % of samples means that a handful of clipped or odd samples cannot hide the padding. The same test catches 16-bit audio stored as 32-bit float, because every sample is then an exact 16-bit value.

## Limitations

No automatic tool can be certain, so please read the reasons and look at the plots for files that matter to you. The following cases are known to be hard.

A source sample rate guess can be off by one family member. A resampler with a soft transition (such as FFmpeg's default `swr`) smears the 22.05 kHz edge upward, so a 44.1 kHz source can be reported as 48 kHz. The verdict is still FAKE, but the source rate is approximate.

A 16-bit master that was converted to 24-bit with a gain change, dither or resampling fills the low bits with values, so the bit-depth test passes. If the sample rate was also raised, the spectral test still catches it. Plain "16/44.1 to 24/44.1 with a volume change" is not detected.

Old analogue recordings often have no content above 20 kHz even when transferred at 96 kHz or higher. These appear as SUSPICIOUS rather than REAL or FAKE, because the data cannot prove which one they are.

A real hi-res file whose ultrasonic band contains only analogue tape or microphone hiss is reported as REAL, because that hiss is genuinely captured above 22 kHz.

MP3 files encoded at 320 kbps low-pass at about 20.5 kHz, which overlaps with ordinary CD anti-aliasing filters, so a lossy source is not always detectable in a 44.1 or 48 kHz file. Upsampled to hi-res, such a file is still caught by the cliff test.

## Development

Run the test suite with pytest. The tests generate synthetic real and fake files (genuine 24/96 and 24/192, upsampled 44.1→96, 48→96, 96→192, bit-padded 16-in-24, 16-in-float, MP3-sourced, dull and silent files) with FFmpeg, so no audio files are stored in the repository.

```bash
python3 -m pytest -q
```

The project layout is documented in [CLAUDE.md](CLAUDE.md).

## License

No license has been chosen yet.
