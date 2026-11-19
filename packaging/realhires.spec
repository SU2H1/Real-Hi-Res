# -*- mode: python ; coding: utf-8 -*-
# PyInstaller spec: one folder containing the GUI (RealHiRes) and the CLI
# (realhires-cli), sharing one runtime, plus a bundled FFmpeg in _internal/ffmpeg.
# On macOS the folder is wrapped into RealHiRes.app.
#
# Build:  python packaging/fetch_ffmpeg.py && pyinstaller --noconfirm packaging/realhires.spec
import glob
import os
import re
import sys

ROOT = os.path.abspath(os.path.join(SPECPATH, ".."))
FFMPEG_DIR = os.environ.get("REALHIRES_FFMPEG_BUNDLE", os.path.join(ROOT, "build", "ffmpeg"))
EXT = ".exe" if sys.platform == "win32" else ""
with open(os.path.join(ROOT, "realhires", "__init__.py"), encoding="utf-8") as fh:
    VERSION = re.search(r'__version__ = "([^"]+)"', fh.read()).group(1)

ffmpeg_bins = []
for name in ("ffmpeg", "ffprobe"):
    path = os.path.join(FFMPEG_DIR, name + EXT)
    if os.path.isfile(path):
        ffmpeg_bins.append((path, "ffmpeg"))
    else:
        print(f"WARNING: {path} not found - the package will need FFmpeg installed on the system")
ffmpeg_docs = [(p, "ffmpeg") for p in glob.glob(os.path.join(FFMPEG_DIR, "*.txt"))]

EXCLUDES = ["matplotlib", "scipy", "pandas", "PIL", "IPython", "pytest", "setuptools", "pip"]

gui_a = Analysis([os.path.join(SPECPATH, "entry_gui.py")], pathex=[ROOT],
                 binaries=ffmpeg_bins, datas=ffmpeg_docs, excludes=EXCLUDES)
cli_a = Analysis([os.path.join(SPECPATH, "entry_cli.py")], pathex=[ROOT],
                 excludes=EXCLUDES + ["tkinter", "_tkinter"])

gui_exe = EXE(PYZ(gui_a.pure), gui_a.scripts, [], exclude_binaries=True, name="RealHiRes",
              console=False, upx=False)
# Different name from the GUI on purpose: Windows and macOS file systems are
# case-insensitive, so "realhires" would clash with "RealHiRes".
cli_exe = EXE(PYZ(cli_a.pure), cli_a.scripts, [], exclude_binaries=True, name="realhires-cli",
              console=True, upx=False)

# The GUI must come first: on macOS the first executable becomes the app's main program.
coll = COLLECT(gui_exe, gui_a.binaries, gui_a.datas, cli_exe, cli_a.binaries, cli_a.datas,
               upx=False, name="RealHiRes")

if sys.platform == "darwin":
    app = BUNDLE(coll, name="RealHiRes.app", bundle_identifier="io.github.su2h1.realhires",
                 version=VERSION,
                 info_plist={"NSHighResolutionCapable": True, "CFBundleShortVersionString": VERSION,
                             "CFBundleVersion": VERSION, "LSMinimumSystemVersion": "11.0"})
