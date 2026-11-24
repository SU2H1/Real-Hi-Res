## What's new in 1.12.0

Real-Hi-Res can now download **synced lyrics** for your songs from [LRCLIB](https://lrclib.net), a free and open lyrics database, the way [LRCGET](https://github.com/tranxuanthang/lrcget) does. Select songs and click **Get lyrics…** (or press Ctrl+L, Cmd+L on a Mac), or choose **Edit → Download synced lyrics for all songs…**; no scan is needed. Each song is looked up by the title, artist, album and length in its tags, and its synced lyrics are saved as an `.lrc` file with the same name next to it, which many music players show line by line in time with the music. Songs that LRCLIB only has plain lyrics for get a `.txt`, instrumental songs are marked, and songs that already have an `.lrc` are skipped, so a second run over a whole library only looks up the songs still missing lyrics. When there is no exact match, a search by title and artist accepts only a song with the same title, artist and length (within 2 seconds), which finds songs whose album is named differently. The audio files are never changed, and only the tags and length of each song are sent to lrclib.net.

The new **Lyrics** tab of the dashboard shows the lyrics of the selected song. Settings has a new **Lyrics** tab with the defaults. On the command line, `--lyrics-only` downloads lyrics without analysing the files, `--lyrics` does it after a scan, and `--check-lyrics-server` tests the connection. Requests are sent one at a time, as LRCLIB asks of programs, so a library of 1,000 songs without lyrics takes roughly 15 to 25 minutes; the window and the command line estimate the time left.

When **Fix to real resolution** has finished, a pop-up now says so. It tells you how many files were fixed (and how many could not be, with the reason), how much smaller they are, where the originals went (for example the "Real-Hi-Res backups" folder, with its full path), what the re-check found, and how many fake files were left alone on purpose. **Show in folder** opens the fixed files. The pop-up does not block the app, and it also appears when you closed the Fix window while it was still working: the files are finished in the background and the list is updated as before.

Some files are still marked fake after fixing and a new scan, and the app now says why. The song list has a new **After fix** column next to **Bandwidth** (and each song's facts an **After fix** line) that says in a few words what **Fix to real resolution** does with the song, for example "Fixable to 16/44.1" or "Stays fake: MP3/AAC source". The details of every fake file end with what **Fix to real resolution** does with it and whether it stays marked fake afterwards: a file made from an MP3/AAC stays fake lossless at any resolution, because nothing can bring back what the lossy encoder removed; a file whose sample rate had to be kept, so as not to cut real content, stays fake hi-res; and files Fix leaves alone say so. The command-line tool shows the same with `-v`.

Real-Hi-Res now also comes as a **Rust version**, released as a preview next to the usual packages. It is a complete rewrite of the desktop app and the command-line tool in the Rust programming language, with every feature of the Python version: the live scan with the time left, the album and artist views, the dashboard with spectrogram and spectrum, saved results, Find, Settings, Fix to real resolution, including bit-exact restores and gapless albums, and everything new in this release described above, from the synced lyrics to the **After fix** column and the pop-up after fixing. It judges every file exactly as the Python version does, with the same verdicts and the same reasons word for word, and it looks up and saves lyrics exactly as the Python version does; an automatic comparison of the two versions on every build makes sure of that. Both versions use the same settings and the same saved results, so you can switch between them, or use both, and a file checked by one is not analysed again by the other.

The Rust version is a native program: it needs no Python inside, starts at once, and its downloads are smaller apart from the bundled FFmpeg. Scans are somewhat faster (about 1.4 times on the test files), because most of the time goes into FFmpeg decoding the audio, which both versions share. Its windows are drawn by the program itself, so it looks a little different, and you can drop folders onto it. As a preview it may still have rough edges; the Python version remains the main one and is unchanged in this release.

## Download

Pick the package for your computer, unpack it and run it. Python, NumPy and FFmpeg are bundled, so nothing else has to be installed.

| System | Download | Start the app |
| --- | --- | --- |
| Windows 10/11, 64-bit | `RealHiRes-<version>-windows-x64.zip` | `RealHiRes\RealHiRes.exe` |
| macOS 12+ on Apple Silicon (M1 and later) | `RealHiRes-<version>-macos-arm64.dmg` | drag `RealHiRes` to Applications |
| macOS 10.15+ on Intel | `RealHiRes-<version>-macos-x86_64.dmg` | drag `RealHiRes` to Applications |
| Linux x86-64 | `RealHiRes-<version>-linux-x64.tar.gz` | `RealHiRes/RealHiRes` (optional: `RealHiRes/install.sh` adds a menu entry) |
| Linux ARM64 (e.g. Raspberry Pi 4/5 with 64-bit OS) | `RealHiRes-<version>-linux-arm64.tar.gz` | `RealHiRes/RealHiRes` (optional: `RealHiRes/install.sh`) |

**Rust version (preview).** The same app and command-line tool, rewritten in Rust (see "What's new" above), with FFmpeg bundled:

| System | Download | Start the app |
| --- | --- | --- |
| Windows 10/11, 64-bit | `RealHiRes-Rust-<version>-windows-x64.zip` | `RealHiRes-Rust\realhires-gui.exe` |
| macOS 12+ on Apple Silicon | `RealHiRes-Rust-<version>-macos-arm64.dmg` | drag `RealHiRes-Rust` to Applications |
| macOS 10.15+ on Intel | `RealHiRes-Rust-<version>-macos-x86_64.dmg` | drag `RealHiRes-Rust` to Applications |
| Linux x86-64 | `RealHiRes-Rust-<version>-linux-x64.tar.gz` | `RealHiRes-Rust/realhires-gui` (optional: `RealHiRes-Rust/install.sh`) |
| Linux ARM64 | `RealHiRes-Rust-<version>-linux-arm64.tar.gz` | `RealHiRes-Rust/realhires-gui` (optional: `RealHiRes-Rust/install.sh`) |

Its command-line tool is `realhires` (`realhires.exe`) next to the app, with the same options as `realhires-cli`.

The `.zip` files for macOS contain the same app as the `.dmg` files. The Linux packages need glibc 2.28 or newer (any distribution from 2018 onwards), and the Rust version's app also needs OpenGL, which every desktop has. Every package also contains the command-line tool. On Windows, keep the `.pkg` files and the `_internal` folder next to the `.exe` files of the Python version, and the `ffmpeg` folder next to those of the Rust version.

**First launch.** The apps are not code-signed. On Windows, SmartScreen may ask you to confirm with **More info → Run anyway**. On macOS 15 and later, open **System Settings → Privacy & Security** and click **Open Anyway**. On older macOS, right-click the app and choose **Open**, or run `xattr -dr com.apple.quarantine /Applications/RealHiRes.app` (or `/Applications/RealHiRes-Rust.app`) once in Terminal.

The bundled FFmpeg executables are licensed separately (LGPL on Windows and Linux, GPL on macOS). See `FFMPEG-NOTICE.txt` inside the package.
