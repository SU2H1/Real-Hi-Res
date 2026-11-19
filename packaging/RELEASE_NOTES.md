## Download

Pick the package for your computer, unpack it and run it. Python, NumPy and FFmpeg are bundled, so nothing else has to be installed.

| System | Download | Start the app |
| --- | --- | --- |
| Windows 10/11, 64-bit | `RealHiRes-windows-x64.zip` | `RealHiRes\RealHiRes.exe` |
| macOS 12+ on Apple Silicon (M1 and later) | `RealHiRes-macos-arm64.dmg` | drag `RealHiRes` to Applications |
| macOS 10.15+ on Intel | `RealHiRes-macos-x86_64.dmg` | drag `RealHiRes` to Applications |
| Linux x86-64 | `RealHiRes-linux-x64.tar.gz` | `RealHiRes/RealHiRes` (optional: `RealHiRes/install.sh` adds a menu entry) |
| Linux ARM64 (e.g. Raspberry Pi 4/5 with 64-bit OS) | `RealHiRes-linux-arm64.tar.gz` | `RealHiRes/RealHiRes` (optional: `RealHiRes/install.sh`) |

The `.zip` files for macOS contain the same app as the `.dmg` files. The Linux packages need glibc 2.28 or newer (any distribution from 2018 onwards), and every package also contains the command-line tool `realhires-cli`.

**First launch.** The apps are not code-signed. On Windows, SmartScreen may ask you to confirm with **More info → Run anyway**. On macOS 15 and later, open **System Settings → Privacy & Security** and click **Open Anyway**. On older macOS, right-click the app and choose **Open**, or run `xattr -dr com.apple.quarantine /Applications/RealHiRes.app` once in Terminal.

The bundled FFmpeg executables are licensed separately (LGPL on Windows and Linux, GPL on macOS). See `FFMPEG-NOTICE.txt` inside the package.
