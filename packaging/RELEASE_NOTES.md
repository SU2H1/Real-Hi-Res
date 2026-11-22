## What's new in 1.11.0

While a scan runs, the status line now estimates how long it will take to finish ("Scanning: 120/500 files, ... about 6 min left"). The estimate appears after the first few files and is based on how much audio has been analysed so far, so libraries that mix short and long tracks, or CD-quality and 24/192 files, are estimated fairly, and files whose saved result is reused are left out of it. The command-line tool shows the same estimate on its progress line.

Genuine 24-bit files at 44.1 or 48 kHz now say why they count as hi-res: their **Why** reads "Genuine 24-bit at standard 44.1 kHz: hi-res by bit depth only", and the details explain that their sample rate is a standard one, so only the bit depth goes beyond CD quality. Results saved by earlier versions show the new text too, without a re-scan.

The **Antivirus scan results** below show in the Kaspersky column only Kaspersky's own verdict, from its Threat Intelligence Portal, and each result links to Kaspersky's page. A file Kaspersky has not judged yet says so; the column no longer shows Kaspersky's result as reported by VirusTotal. The daily refresh also sends the downloads to Kaspersky, so it can judge them. Release pages now describe only the changes of their own version.

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
