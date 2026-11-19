## Download

Pick the package for your computer, unzip it and run it. Python and FFmpeg are bundled, so nothing else has to be installed.

| System | File | Start the GUI | Command line |
| --- | --- | --- | --- |
| Windows 10/11 (64-bit) | `RealHiRes-windows-x64.zip` | `RealHiRes\RealHiRes.exe` | `RealHiRes\realhires-cli.exe "D:\Music"` |
| macOS 11+ (Apple Silicon) | `RealHiRes-macos-arm64.zip` | `RealHiRes.app` | `RealHiRes.app/Contents/MacOS/realhires-cli ~/Music` |
| Linux x86-64 (glibc 2.35+) | `RealHiRes-linux-x64.tar.gz` | `RealHiRes/RealHiRes` | `RealHiRes/realhires-cli ~/Music` |

The app is not code-signed. On Windows, SmartScreen may ask you to confirm with **More info → Run anyway**. On macOS, right-click `RealHiRes.app` and choose **Open**, or run `xattr -dr com.apple.quarantine RealHiRes.app` once. Intel Macs can run Real-Hi-Res from source (see the README).

The bundled FFmpeg executables are licensed separately (LGPL on Windows/Linux, GPL on macOS). See `FFMPEG-NOTICE.txt` inside the package.
