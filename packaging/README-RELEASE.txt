Real-Hi-Res - fake hi-res audio detector
=========================================

This package is self-contained: Python, NumPy and FFmpeg are bundled, so
nothing else needs to be installed.

Start the app
-------------
  Windows : double-click RealHiRes.exe
  macOS   : drag RealHiRes.app into Applications, then open it
            (see "First launch on macOS" below)
  Linux   : run ./RealHiRes
            Optional: ./install.sh adds Real-Hi-Res to your application menu
            and links realhires-cli into ~/.local/bin (no root needed).
            ./uninstall.sh removes that again.  The app runs from this
            folder, so re-run install.sh if you move the folder.

Click "Add Folder..." (or "Add Files...").  Every folder/file you add appears
in the "Sources to scan" panel with the number of audio files found (red if
none), and the files are listed right away as "Not scanned".  Then press
"Scan".  While it runs, the status line counts the files and, after the
first few, estimates the time left ("about 6 min left").  After the scan the list shows the problem files: FAKE LOSSLESS ("fake FLAC" made from an MP3/AAC), FAKE HI-RES
(upsampled or 16-bit padded) and SUSPICIOUS, each with a one-line reason; tick
the other boxes to see everything.  After the scan a summary window shows the
percentage of files per verdict as a pie chart (click a slice to list
only those files).

Click a row for the dashboard: verdict, reason, key facts, spectrogram and
average spectrum.  "Open Folder" (or double-click / Enter) shows the file in
your file manager, "Open File" (Ctrl/Cmd+Enter) plays it.  "View: By album /
By artist" shows your library as cover-art cards with a ring showing how many
songs are fake; click a card to list its songs.  Ctrl/Cmd+F finds text like a
web browser (Enter = next).  "Fix to real
resolution..." (Ctrl/Cmd+Shift+F) corrects selected fake files in place so their
properties show the real resolution (e.g. a fake 24/96 becomes 16/44.1), keeping
name, tags and cover art; originals go to a "Real-Hi-Res backups" folder.
Ctrl/Cmd+A selects every listed file: tick only "Fake lossless" and "Fake
hi-res", press Ctrl/Cmd+A, then "Fix to real resolution..." to fix them all.
Settings > Fixing > "After a scan" can ask, or fix the fake files on its own,
when a scan finishes.  Fake FLACs made from an MP3 that are already 16/44.1
cannot be improved; the Fix window lists them last and says why.
"Get lyrics..." (Ctrl/Cmd+L, or Edit > Download synced lyrics for all songs)
downloads synced lyrics from LRCLIB (lrclib.net) and saves them as <name>.lrc
next to each song, as LRCGET does (plain lyrics as <name>.txt); no scan is
needed.  Songs are looked up by their title/artist/album tags and length, one
at a time as LRCLIB asks, and songs that already have an .lrc are skipped.
The Lyrics tab of the dashboard shows them.
"Settings..." lets you change detection thresholds, scanning options, display
and fixing preferences; they are saved and also used by the command-line tool.

Results are saved: a file that has not changed is never analysed again, not
on the next scan, after restarting, or after updating Real-Hi-Res.  A folder
you checked before shows its verdicts as soon as you add it, and "Scan" only
analyses new or changed files.  Right-click > "Re-scan" analyses a file again.
The results are kept in your user folder (Settings > Saved results shows where
and can delete them); the command-line tool uses them too (--rescan,
--no-cache, --forget-results).

Command line
------------
  Windows : realhires-cli.exe "D:\Music"
  macOS   : /Applications/RealHiRes.app/Contents/MacOS/realhires-cli ~/Music
  Linux   : ./realhires-cli ~/Music     (or just realhires-cli after install.sh)

  Run with --help for all options (--only problems, --csv, --json,
  --spectrogram DIR, --fix, --fix-dir DIR, --lyrics, --lyrics-only,
  --set NAME=VALUE, -v ...).

First launch on macOS
---------------------
The app is not signed with an Apple Developer ID, so macOS blocks the first
launch ("damaged" / "cannot be opened").
  * macOS 15 Sequoia and later: System Settings > Privacy & Security, scroll
    down and click "Open Anyway" next to the RealHiRes message.
  * Older macOS: right-click RealHiRes.app > Open.
  * Any version, in Terminal:
      xattr -dr com.apple.quarantine /Applications/RealHiRes.app

First launch on Windows
-----------------------
SmartScreen may warn about an unrecognised app.  Click "More info" > "Run
anyway".  Keep the whole folder together: RealHiRes.exe and realhires-cli.exe
are small launchers, and the program itself is in RealHiRes.pkg,
realhires-cli.pkg and the _internal folder next to them.  If an antivirus
program reports a trojan, it is a false positive caused by the PyInstaller
packaging (see the README on GitHub); restore the file and report it to the
vendor.

Linux requirements
------------------
glibc 2.28 or newer (any distribution from 2018 onwards) and, for the app, a
graphical desktop (X11, or Wayland with XWayland).  The command-line tool also
works on servers without a desktop.

Full documentation: https://github.com/SU2H1/Real-Hi-Res
FFmpeg licence information: see FFMPEG-NOTICE.txt in the "ffmpeg" folder
inside the package (Windows/Linux: _internal/ffmpeg; macOS: inside the app,
right-click RealHiRes.app > Show Package Contents > Contents).
