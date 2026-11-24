## What's new in 1.12.0

Real-Hi-Res can now download **synced lyrics** for your songs from [LRCLIB](https://lrclib.net), a free and open lyrics database, the way [LRCGET](https://github.com/tranxuanthang/lrcget) does. Select songs and click **Get lyrics…** (or press Ctrl+L, Cmd+L on a Mac), or choose **Edit → Download synced lyrics for all songs…**; no scan is needed. Each song is looked up by the title, artist, album and length in its tags, and its synced lyrics are saved as an `.lrc` file with the same name next to it, which many music players show line by line in time with the music. Songs that LRCLIB only has plain lyrics for get a `.txt`, instrumental songs are marked, and songs that already have an `.lrc` are skipped, so a second run over a whole library only looks up the songs still missing lyrics. When there is no exact match, a search by title and artist accepts only a song with the same title, artist and length (within 2 seconds), which finds songs whose album is named differently. The audio files are never changed, and only the tags and length of each song are sent to lrclib.net.

The new **Lyrics** tab of the dashboard shows the lyrics of the selected song. Settings has a new **Lyrics** tab with the defaults. On the command line, `--lyrics-only` downloads lyrics without analysing the files, `--lyrics` does it after a scan, and `--check-lyrics-server` tests the connection. Requests are sent one at a time, as LRCLIB asks of programs, so a library of 1,000 songs without lyrics takes roughly 15 to 25 minutes; the window and the command line estimate the time left.

## Download

Pick the package for your computer, unpack it and run it. Python, NumPy and FFmpeg are bundled, so nothing else has to be installed.

| System | Download | Start the app |
| --- | --- | --- |
| Windows 10/11, 64-bit | `RealHiRes-<version>-windows-x64.zip` | `RealHiRes\RealHiRes.exe` |
| macOS 12+ on Apple Silicon (M1 and later) | `RealHiRes-<version>-macos-arm64.dmg` | drag `RealHiRes` to Applications |
| macOS 10.15+ on Intel | `RealHiRes-<version>-macos-x86_64.dmg` | drag `RealHiRes` to Applications |
| Linux x86-64 | `RealHiRes-<version>-linux-x64.tar.gz` | `RealHiRes/RealHiRes` (optional: `RealHiRes/install.sh` adds a menu entry) |
| Linux ARM64 (e.g. Raspberry Pi 4/5 with 64-bit OS) | `RealHiRes-<version>-linux-arm64.tar.gz` | `RealHiRes/RealHiRes` (optional: `RealHiRes/install.sh`) |

The `.zip` files for macOS contain the same app as the `.dmg` files. The Linux packages need glibc 2.28 or newer (any distribution from 2018 onwards), and every package also contains the command-line tool `realhires-cli`. On Windows, keep the `.pkg` files and the `_internal` folder next to the `.exe` files.

**First launch.** The apps are not code-signed. On Windows, SmartScreen may ask you to confirm with **More info → Run anyway**. On macOS 15 and later, open **System Settings → Privacy & Security** and click **Open Anyway**. On older macOS, right-click the app and choose **Open**, or run `xattr -dr com.apple.quarantine /Applications/RealHiRes.app` once in Terminal.

The bundled FFmpeg executables are licensed separately (LGPL on Windows and Linux, GPL on macOS). See `FFMPEG-NOTICE.txt` inside the package.
