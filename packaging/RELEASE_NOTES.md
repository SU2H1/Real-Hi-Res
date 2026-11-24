## What's new in 1.12.0

When **Fix to real resolution** has finished, a pop-up now says so. It tells you how many files were fixed (and how many could not be, with the reason), how much smaller they are, where the originals went (for example the "Real-Hi-Res backups" folder, with its full path), what the re-check found, and how many fake files were left alone on purpose. **Show in folder** opens the fixed files. The pop-up does not block the app, and it also appears when you closed the Fix window while it was still working: the files are finished in the background and the list is updated as before.

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
