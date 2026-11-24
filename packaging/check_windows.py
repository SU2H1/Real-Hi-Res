"""Check the Windows executables for the traits that antivirus heuristics count
against PyInstaller programs, and fail the build if one is back.

    python packaging/check_windows.py dist/UniversalMusicPlayer

* Each .exe must be only the launcher: the compressed program archive has to be
  the separate <name>.pkg next to it (spec: append_pkg=False), so the .exe is small
  and not one big high-entropy blob.
* Each .exe must carry version information (product, company, description).
* The script also prints the SHA-256 of the PyInstaller launcher that was used, so
  the CI log shows that it was compiled on the build machine (PYINSTALLER_COMPILE_
  BOOTLOADER) rather than taken from the prebuilt wheel, whose launcher is shared
  by every PyInstaller program, malware included.
"""

import hashlib
import math
import os
import sys
from collections import Counter

MAX_EXE_BYTES = 3 * 1024 * 1024
EXES = {"UniversalMusicPlayer": "Universal Music Player", "realhires-cli": "Universal Music Player"}


def entropy(data: bytes) -> float:
    if not data:
        return 0.0
    n = len(data)
    return -sum(c / n * math.log2(c / n) for c in Counter(data).values())


def version_strings(path: str) -> dict:
    import pefile  # a PyInstaller dependency on Windows

    pe = pefile.PE(path, fast_load=False)
    out = {}
    for info in getattr(pe, "FileInfo", None) or []:
        for entry in info:
            for table in getattr(entry, "StringTable", []) or []:
                out.update({k.decode("utf-8", "replace"): v.decode("utf-8", "replace")
                            for k, v in table.entries.items()})
    pe.close()
    return out


def main(folder: str) -> int:
    problems = []
    for name, product in EXES.items():
        exe = os.path.join(folder, name + ".exe")
        pkg = os.path.join(folder, name + ".pkg")
        data = open(exe, "rb").read()
        print(f"{name}.exe: {len(data):,} bytes, entropy {entropy(data):.2f} bits/byte, "
              f"sha256 {hashlib.sha256(data).hexdigest()}")
        if not os.path.isfile(pkg):
            problems.append(f"{name}.pkg is missing: the program archive is inside the .exe again")
        if len(data) > MAX_EXE_BYTES:
            problems.append(f"{name}.exe is {len(data):,} bytes: expected only the launcher")
        strings = version_strings(exe)
        print(f"  version info: {strings}")
        if strings.get("ProductName") != product or not strings.get("FileDescription"):
            problems.append(f"{name}.exe has no proper version information")
    try:
        import PyInstaller

        boot = os.path.join(os.path.dirname(PyInstaller.__file__), "bootloader", "Windows-64bit-intel")
        for f in sorted(os.listdir(boot)):
            if f.endswith(".exe"):
                h = hashlib.sha256(open(os.path.join(boot, f), "rb").read()).hexdigest()
                print(f"PyInstaller {PyInstaller.__version__} launcher {f}: sha256 {h}")
    except (ImportError, OSError) as exc:
        print(f"(could not list the PyInstaller launchers: {exc})")
    for p in problems:
        print("ERROR:", p, file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1] if len(sys.argv) > 1 else "dist/UniversalMusicPlayer"))
