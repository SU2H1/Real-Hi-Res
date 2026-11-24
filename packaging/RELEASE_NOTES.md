## What's new in 1.12.1

Double-clicking the command-line tool now opens the app. The Windows packages contain two programs each: the desktop app (`realhires-gui.exe` in the Rust version, `RealHiRes.exe` in the Python version) and the command-line tool (`realhires.exe` and `realhires-cli.exe`), which checks the folders you name in a terminal. Double-clicked in Explorer, the command-line tool used to open a terminal window that showed a short usage message and nothing else. Now it opens the desktop app next to it instead. If the app is missing, the window says what to start and stays open until you press Enter.

The Rust desktop app also shows a message box when it cannot open its window, for example on a computer whose graphics driver has no OpenGL 2.0, instead of closing without a word.

Nothing else has changed since 1.12.0. Its release page describes the synced lyrics from LRCLIB, the After fix column, the pop-up after fixing and the new Rust version.

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

## Credits

Synced lyrics come from [LRCLIB](https://lrclib.net) and the people who contribute to it, and the lyrics feature is modelled on LRCGET by tranxuanthang ([github.com/tranxuanthang/lrcget](https://github.com/tranxuanthang/lrcget)), who also makes LRCLIB. Both are MIT-licensed, and Real-Hi-Res's own lyrics code contains no code from either. Both apps credit them in the lyrics window and in Help → About. Thank you to them.

The bundled FFmpeg executables are licensed separately (LGPL on Windows and Linux, GPL on macOS). See `FFMPEG-NOTICE.txt` inside the package.
