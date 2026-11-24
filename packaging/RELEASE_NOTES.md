## What's new in 1.12.0

Real-Hi-Res now also comes as a **Rust version**, released as a preview next to the usual packages. It is a complete rewrite of the desktop app and the command-line tool in the Rust programming language, with every feature of the Python version: the live scan with the time left, the album and artist views, the dashboard with spectrogram and spectrum, saved results, Find, Settings, and Fix to real resolution, including bit-exact restores and gapless albums. It judges every file exactly as the Python version does, with the same verdicts and the same reasons word for word; an automatic comparison of the two versions on every build makes sure of that. Both versions use the same settings and the same saved results, so you can switch between them, or use both, and a file checked by one is not analysed again by the other.

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
