Real-Hi-Res (Rust version) - fake hi-res audio detector
=======================================================

This is the Rust version of Real-Hi-Res, a preview released next to the
Python version.  It finds the same fakes, with the same verdicts and reasons,
and it shares its settings and saved results with the Python version: a file
checked by one of them is not analysed again by the other.  FFmpeg is
bundled, so nothing else needs to be installed.

Start the app
-------------
  Windows : double-click realhires-gui.exe
  macOS   : drag RealHiRes-Rust.app into Applications, then open it
            (see "First launch on macOS" below)
  Linux   : run ./realhires-gui
            Optional: ./install.sh adds "Real-Hi-Res (Rust)" to your
            application menu and links the command-line tool into
            ~/.local/bin as realhires-rust (no root needed).
            ./uninstall.sh removes that again.  The app runs from this
            folder, so re-run install.sh if you move the folder.

Click "Add Folder..." or "Add Files..." (or drop folders onto the window).
Every folder or file you add appears under "Sources to scan" with the number
of audio files found, and the files are listed at once as "Not scanned";
files checked before show their saved verdict straight away.  Then press
"Scan".  The status line counts the files and estimates the time left.
Afterwards the list shows the problem files: FAKE LOSSLESS ("fake FLAC" made
from an MP3/AAC), FAKE HI-RES (upsampled or 16-bit padded) and SUSPICIOUS,
each with a one-line reason; tick the other boxes to see everything.  A
summary window shows the share of each verdict (click a slice to list only
those files).

Click a row for the dashboard: verdict, reason, key facts, spectrogram and
average spectrum.  "Open Folder" (or double-click / Enter) shows the file in
your file manager, "Open File" (Ctrl/Cmd+Enter) plays it.  "View: By album /
By artist" shows your library as cover-art cards with a ring showing how
many songs are fake; click a card to list its songs.  Ctrl/Cmd+F finds text
(Enter = next, Esc = close).  "Fix to real resolution..." (Ctrl/Cmd+Shift+F)
corrects the selected fake files in place so their properties show the real
resolution, keeping name, tags and cover art; originals go to a
"Real-Hi-Res backups" folder.  Ctrl/Cmd+A selects every listed file.
"Settings..." holds the detection thresholds, scanning, saved results,
display and fixing options.

Command line
------------
  Windows : realhires.exe "D:\Music"
  macOS   : /Applications/RealHiRes-Rust.app/Contents/MacOS/realhires ~/Music
  Linux   : ./realhires ~/Music     (or realhires-rust after install.sh)

  The options are those of the Python version's realhires-cli: run with
  --help (--only problems, --csv, --json, --spectrogram DIR, --fix,
  --fix-dir DIR, --set NAME=VALUE, -v ...).

First launch on macOS
---------------------
The app is not signed with an Apple Developer ID, so macOS blocks the first
launch ("damaged" / "cannot be opened").
  * macOS 15 Sequoia and later: System Settings > Privacy & Security, scroll
    down and click "Open Anyway" next to the message about the app.
  * Older macOS: right-click RealHiRes-Rust.app > Open.
  * Any version, in Terminal:
      xattr -dr com.apple.quarantine /Applications/RealHiRes-Rust.app

First launch on Windows
-----------------------
SmartScreen may warn about an unrecognised app.  Click "More info" > "Run
anyway".  Keep the "ffmpeg" folder next to the two programs.

Linux requirements
------------------
glibc 2.28 or newer (any distribution from 2018 onwards) and, for the app, a
graphical desktop with OpenGL (X11 or Wayland).  The command-line tool also
works on servers without a desktop.

Full documentation: https://github.com/SU2H1/Real-Hi-Res
FFmpeg licence information: see FFMPEG-NOTICE.txt (Windows/Linux: in the
"ffmpeg" folder; macOS: inside the app, right-click > Show Package Contents >
Contents > Resources).
