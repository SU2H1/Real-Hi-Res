"""Verify that a built package runs on the systems we claim to support.

Linux : every ELF file's required GLIBC symbol versions (via `objdump -T`) must
        be <= --max-glibc, so the package runs on that glibc and newer.
macOS : every Mach-O file's minimum macOS (LC_BUILD_VERSION / LC_VERSION_MIN_MACOSX,
        via `otool -l`) must be <= --max-macos, and every file must contain the
        --arch slice (arm64 / x86_64), so nothing needs Rosetta or a newer OS.

Usage:
  python packaging/check_compat.py dist/RealHiRes --max-glibc 2.28
  python packaging/check_compat.py dist/RealHiRes.app --max-macos 12.0 --arch arm64
"""

import argparse
import os
import re
import subprocess
import sys
from typing import Iterator, List, Optional, Tuple

ELF_MAGIC = b"\x7fELF"
MACHO_MAGICS = {b"\xfe\xed\xfa\xce", b"\xce\xfa\xed\xfe", b"\xfe\xed\xfa\xcf",
                b"\xcf\xfa\xed\xfe", b"\xca\xfe\xba\xbe", b"\xbe\xba\xfe\xca"}


def vtuple(v: str) -> Tuple[int, ...]:
    return tuple(int(p) for p in v.split("."))


def files_with_magic(root: str, magics) -> Iterator[str]:
    for d, _, names in os.walk(root):
        for n in names:
            p = os.path.join(d, n)
            if os.path.islink(p) or not os.path.isfile(p):
                continue
            try:
                with open(p, "rb") as f:
                    head = f.read(4)
            except OSError:
                continue
            if head in magics:
                yield p


def run(cmd: List[str]) -> str:
    return subprocess.run(cmd, capture_output=True, text=True, errors="replace").stdout


def glibc_needed(path: str) -> Optional[str]:
    found = re.findall(r"GLIBC_(\d+(?:\.\d+)+)", run(["objdump", "-T", path]))
    return max(found, key=vtuple) if found else None


def macos_min(path: str) -> Optional[str]:
    out = run(["otool", "-l", path])
    found = re.findall(r"\bminos (\d+(?:\.\d+)*)", out)
    found += re.findall(r"LC_VERSION_MIN_MACOSX\s+cmdsize \d+\s+version (\d+(?:\.\d+)*)", out)
    return max(found, key=vtuple) if found else None


def archs(path: str) -> List[str]:
    return run(["lipo", "-archs", path]).split()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("root")
    ap.add_argument("--max-glibc")
    ap.add_argument("--max-macos")
    ap.add_argument("--arch", help="macOS: architecture every binary must contain")
    a = ap.parse_args()
    problems: List[str] = []
    worst: Tuple[Tuple[int, ...], str, str] = ((0,), "", "")

    if a.max_glibc:
        limit = vtuple(a.max_glibc)
        n = 0
        for p in files_with_magic(a.root, {ELF_MAGIC}):
            n += 1
            v = glibc_needed(p)
            if v and vtuple(v) > worst[0]:
                worst = (vtuple(v), v, p)
            if v and vtuple(v) > limit:
                problems.append(f"{p} needs GLIBC_{v} (> {a.max_glibc})")
        print(f"Checked {n} ELF files; highest glibc required: {worst[1] or 'none'} ({worst[2]})")

    if a.max_macos:
        limit = vtuple(a.max_macos)
        n = 0
        for p in files_with_magic(a.root, MACHO_MAGICS):
            n += 1
            v = macos_min(p)
            if v and vtuple(v) > worst[0]:
                worst = (vtuple(v), v, p)
            if v and vtuple(v) > limit:
                problems.append(f"{p} needs macOS {v} (> {a.max_macos})")
            if a.arch and a.arch not in archs(p):
                problems.append(f"{p} has no {a.arch} code (archs: {' '.join(archs(p)) or '?'})")
        print(f"Checked {n} Mach-O files; highest macOS required: {worst[1] or 'none'} ({worst[2]})")

    if problems:
        shown = problems[:25] + ([f"... and {len(problems) - 25} more"] if len(problems) > 25 else [])
        print(f"{len(problems)} compatibility problem(s):", *shown, sep="\n  ", file=sys.stderr)
        return 1
    print("Compatibility check passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
