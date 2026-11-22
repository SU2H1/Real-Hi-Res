## What's new in 1.11.0

While a scan runs, the status line now estimates how long it will take to finish ("Scanning: 120/500 files, ... about 6 min left"). The estimate appears after the first few files and is based on how much audio has been analysed so far, so libraries that mix short and long tracks, or CD-quality and 24/192 files, are estimated fairly, and files whose saved result is reused are left out of it. The command-line tool shows the same estimate on its progress line.

On the release page, the Kaspersky column now says "on VirusTotal" when its result is Kaspersky's verdict as reported by VirusTotal, because its link then leads to VirusTotal rather than to Kaspersky's own site. The daily antivirus refresh also sends the downloads to Kaspersky's Threat Intelligence Portal, so Kaspersky's own page soon has a verdict for every file.

## What's new in 1.10.0

Typing in the Search box no longer freezes the window. The list used to be rebuilt on every key you typed or deleted, which made big libraries stutter; it is now filtered once you pause typing, and the rebuild itself is about three times faster.

**Settings → Fixing → After a scan** can ask whether to fix the fake files once a scan has finished, or fix them automatically (the default stays "Do nothing"). Originals still go to the backup folder.

The Fix window now explains why it lists fewer files than were flagged as fake: under the options it counts the files it leaves unchanged by reason, and lists them last, in grey. Most are "fake FLACs" made from an MP3 that are already 16-bit/44.1 kHz: there is nothing to convert, and MP3 damage cannot be undone. The command-line `--fix` prints the same summary.

Fixed: starting a new scan right after one had finished could show a stray "Nothing scanned yet" message, which on macOS froze the window until it was closed.

## What's new in 1.9.0

Scan results are saved, so your library is never analysed twice. A file that has not changed since it was checked is not analysed again: not on the next scan, not after restarting the app, and not after updating Real-Hi-Res to this or any later version. Add a folder you checked before and its verdicts appear immediately, before you even press Scan; Scan then analyses only new and changed files. Moved or renamed files keep their results, and the spectrogram is saved too. If a future update improves the detection rules, your saved results stay and are marked, and **Re-scan them** analyses those files with the new rules whenever you want. Right-click → **Re-scan** analyses any file again. The results are kept in your user folder; **Settings → Saved results** shows where, how big they are, and can delete them. The command-line tool shares them (`--rescan`, `--rescan-outdated`, `--no-cache`, `--forget-results`).

Releases are now scanned with VirusTotal, Kaspersky and Hybrid Analysis before they are published, so the **Antivirus scan results** below are there from the first minute.

## What's new in 1.8.1

The download files now carry the version in their names, for example `RealHiRes-<version>-windows-x64.zip`. The **Antivirus scan results** section below lists every file with its current VirusTotal, Kaspersky and Hybrid Analysis result; it is updated every day and explains any detection.

The Windows package no longer looks like a packed trojan to antivirus heuristics, which made a few engines flag earlier builds as false positives. Its launcher is now compiled on the build machine instead of being PyInstaller's widely shared prebuilt one, the compressed program sits in separate `.pkg` files next to the small `.exe` launchers, and both executables carry version information and uncompressed icons. Keep the `.pkg` files and the `_internal` folder next to the `.exe` files.

## What's new in 1.8.0

"Fix to real resolution" is safer and more faithful. It never lowers the sample rate below a point where the music is still present, so a doubtful estimate errs towards keeping a higher rate instead of cutting anything. Fake hi-res files that FFmpeg upsampled by a whole-number ratio (for example 48 → 192 kHz) are turned back into the original 16-bit master bit for bit, without any resampling. Gapless albums (live recordings, DJ mixes, classical works) stay gapless: each track is converted together with the audio around it, so there is no click at the joins.

## What's new in 1.7.0

Detection is more accurate in the three cases that were known to be hard. The source sample rate of an upsampled file is now found reliably for FFmpeg's default resampler as well, so "Fix to real resolution" converts such files back to 44.1 kHz instead of 48 kHz, and files that this soft resampler upsampled from 48 kHz are no longer passed as real. A new value-grid test catches 16-bit masters that were converted to 24-bit or float with only a volume change or dither. A new converter-noise-floor test proves that a dull or old analogue recording was really digitised at its high sample rate, so such files are now reported as real instead of suspicious when the evidence is there. Fix now restores padded and dithered 16-bit masters bit-exactly. Ctrl+A (Cmd+A on a Mac) selects every listed file, so you can tick only "Fake lossless" and "Fake hi-res", press Ctrl+A and fix them all at once. Rescan your library before fixing, because older scans used the less accurate source-rate estimate. See the README for how each test works.

## Download

Pick the package for your computer, unpack it and run it. Python, NumPy and FFmpeg are bundled, so nothing else has to be installed.

| System | Download | Start the app |
| --- | --- | --- |
| Windows 10/11, 64-bit | `RealHiRes-<version>-windows-x64.zip` | `RealHiRes\RealHiRes.exe` |
| macOS 12+ on Apple Silicon (M1 and later) | `RealHiRes-<version>-macos-arm64.dmg` | drag `RealHiRes` to Applications |
| macOS 10.15+ on Intel | `RealHiRes-<version>-macos-x86_64.dmg` | drag `RealHiRes` to Applications |
| Linux x86-64 | `RealHiRes-<version>-linux-x64.tar.gz` | `RealHiRes/RealHiRes` (optional: `RealHiRes/install.sh` adds a menu entry) |
| Linux ARM64 (e.g. Raspberry Pi 4/5 with 64-bit OS) | `RealHiRes-<version>-linux-arm64.tar.gz` | `RealHiRes/RealHiRes` (optional: `RealHiRes/install.sh`) |

The `.zip` files for macOS contain the same app as the `.dmg` files. The Linux packages need glibc 2.28 or newer (any distribution from 2018 onwards), and every package also contains the command-line tool `realhires-cli`.

**First launch.** The apps are not code-signed. On Windows, SmartScreen may ask you to confirm with **More info → Run anyway**. On macOS 15 and later, open **System Settings → Privacy & Security** and click **Open Anyway**. On older macOS, right-click the app and choose **Open**, or run `xattr -dr com.apple.quarantine /Applications/RealHiRes.app` once in Terminal.

The bundled FFmpeg executables are licensed separately (LGPL on Windows and Linux, GPL on macOS). See `FFMPEG-NOTICE.txt` inside the package.
