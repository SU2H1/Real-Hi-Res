"""Download static FFmpeg + FFprobe builds for the current platform into build/ffmpeg.

This is run by the release workflow on GitHub's build machines so that the
release packages work without a separate FFmpeg installation.

Sources (both publish ffmpeg *and* ffprobe as stand-alone executables):
  * Windows x64, Linux x64 / arm64: BtbN/FFmpeg-Builds, LGPL variant
    https://github.com/BtbN/FFmpeg-Builds
  * macOS arm64:                     Martin Riedl's static builds
    https://ffmpeg.martin-riedl.de
  * macOS x86_64 (Intel):            evermeet.cx static builds
    https://evermeet.cx/ffmpeg/

Usage: python packaging/fetch_ffmpeg.py [--dest DIR]
"""

import argparse
import io
import os
import platform
import shutil
import stat
import sys
import tarfile
import urllib.request
import zipfile

BTBN = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/"
RIEDL = "https://ffmpeg.martin-riedl.de/redirect/latest/macos/{arch}/release/{tool}.zip"
EVERMEET_INFO = "https://evermeet.cx/ffmpeg/info/{tool}/release"

NOTICE = """FFmpeg is bundled with this package as separate, unmodified executables
(ffmpeg, ffprobe) that Real-Hi-Res runs as external programs.

FFmpeg is free software licensed under the LGPL v2.1+ (Windows/Linux builds from
BtbN/FFmpeg-Builds, LGPL variant) or the GPL (macOS builds from
ffmpeg.martin-riedl.de for Apple Silicon and evermeet.cx for Intel).
FFmpeg source code: https://ffmpeg.org/download.html
Build recipes: https://github.com/BtbN/FFmpeg-Builds, https://ffmpeg.martin-riedl.de
and https://evermeet.cx/ffmpeg/
FFmpeg is a trademark of Fabrice Bellard.
"""


def download(url: str) -> bytes:
    print(f"Downloading {url}", flush=True)
    req = urllib.request.Request(url, headers={"User-Agent": "realhires-build"})
    with urllib.request.urlopen(req, timeout=300) as r:
        return r.read()


def make_executable(path: str) -> None:
    os.chmod(path, os.stat(path).st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def _save(src, dest_dir: str, name: str, executable: bool) -> None:
    out_path = os.path.join(dest_dir, name)
    with open(out_path, "wb") as out:
        shutil.copyfileobj(src, out)
    if executable:
        make_executable(out_path)


def fetch_btbn_windows(dest: str) -> None:
    data = download(BTBN + "ffmpeg-master-latest-win64-lgpl.zip")
    with zipfile.ZipFile(io.BytesIO(data)) as z:
        for m in z.namelist():
            base = os.path.basename(m)
            if base in ("ffmpeg.exe", "ffprobe.exe"):
                with z.open(m) as src:
                    _save(src, dest, base, executable=False)
            elif base == "LICENSE.txt":
                with z.open(m) as src:
                    _save(src, dest, "FFMPEG-LICENSE.txt", executable=False)


def fetch_btbn_linux(dest: str, arch: str) -> None:
    data = download(BTBN + f"ffmpeg-master-latest-{arch}-lgpl.tar.xz")
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:xz") as t:
        for m in t.getmembers():
            base = os.path.basename(m.name)
            if not m.isfile():
                continue
            if base in ("ffmpeg", "ffprobe"):
                _save(t.extractfile(m), dest, base, executable=True)
            elif base == "LICENSE.txt":
                _save(t.extractfile(m), dest, "FFMPEG-LICENSE.txt", executable=False)


def fetch_riedl_macos(dest: str, arch: str) -> None:
    for tool in ("ffmpeg", "ffprobe"):
        data = download(RIEDL.format(arch=arch, tool=tool))
        with zipfile.ZipFile(io.BytesIO(data)) as z:
            for m in z.namelist():
                if os.path.basename(m) == tool:
                    with z.open(m) as src:
                        _save(src, dest, tool, executable=True)


def fetch_evermeet_macos(dest: str) -> None:
    """Intel-only static builds that support older macOS than most others."""
    import json

    for tool in ("ffmpeg", "ffprobe"):
        info = json.loads(download(EVERMEET_INFO.format(tool=tool)))
        data = download(info["download"]["zip"]["url"])
        with zipfile.ZipFile(io.BytesIO(data)) as z:
            for m in z.namelist():
                if os.path.basename(m) == tool:
                    with z.open(m) as src:
                        _save(src, dest, tool, executable=True)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--dest", default=os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                                   "..", "build", "ffmpeg"))
    args = ap.parse_args()
    dest = os.path.abspath(args.dest)
    os.makedirs(dest, exist_ok=True)

    machine = platform.machine().lower()
    if sys.platform == "win32":
        fetch_btbn_windows(dest)
    elif sys.platform == "darwin":
        if machine in ("arm64", "aarch64"):
            fetch_riedl_macos(dest, "arm64")
        else:
            fetch_evermeet_macos(dest)
    elif sys.platform.startswith("linux") and machine in ("x86_64", "amd64"):
        fetch_btbn_linux(dest, "linux64")
    elif sys.platform.startswith("linux") and machine in ("aarch64", "arm64"):
        fetch_btbn_linux(dest, "linuxarm64")
    else:
        print(f"No prebuilt FFmpeg configured for {sys.platform}/{machine}", file=sys.stderr)
        return 1
    with open(os.path.join(dest, "FFMPEG-NOTICE.txt"), "w", encoding="utf-8") as f:
        f.write(NOTICE)
    found = sorted(os.listdir(dest))
    print("FFmpeg ready in", dest, "->", found)
    exe = ".exe" if sys.platform == "win32" else ""
    missing = [t for t in ("ffmpeg", "ffprobe") if t + exe not in found]
    if missing:
        print("ERROR: missing " + ", ".join(missing), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
