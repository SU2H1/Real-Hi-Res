Real-Hi-Res - fake hi-res audio detector
=========================================

This package is self-contained: Python, NumPy and FFmpeg are bundled, so
nothing else needs to be installed.

GUI
---
  Windows : double-click RealHiRes.exe
  macOS   : open RealHiRes.app (see "First launch on macOS" below)
  Linux   : run ./RealHiRes

Click "Add Folder..." (or "Add Files..."), then "Scan".  By default the list
shows only files judged FAKE HI-RES or SUSPICIOUS; tick the other boxes to see
everything.  Click a row for the reasons and the spectrum.  Double-click opens
the file's folder.  Use "Export CSV..." / "Export JSON..." to save the list.

Command line
------------
  Windows : realhires-cli.exe "D:\Music"
  macOS   : RealHiRes.app/Contents/MacOS/realhires-cli ~/Music
  Linux   : ./realhires-cli ~/Music

  Run with --help for all options (--only fake, --csv, --json, --plot, -v ...).

First launch on macOS
---------------------
The app is not signed with an Apple Developer ID, so macOS may say it "is
damaged" or "cannot be opened".  Either right-click RealHiRes.app > Open, or
run once in Terminal:

  xattr -dr com.apple.quarantine /path/to/RealHiRes.app

First launch on Windows
-----------------------
SmartScreen may warn about an unrecognised app.  Click "More info" > "Run
anyway".

Full documentation: https://github.com/SU2H1/Real-Hi-Res
FFmpeg licence information: see FFMPEG-NOTICE.txt in the ffmpeg folder.
