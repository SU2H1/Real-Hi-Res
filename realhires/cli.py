"""Command-line interface: `realhires PATH... [options]`."""

from __future__ import annotations

import argparse
import os
import sys
from concurrent.futures import ProcessPoolExecutor
from functools import partial
from typing import Iterable, List

from . import __version__
from .probe import ToolMissingError, require_tools
from .report import eprint, header, row, summary, use_color, write_csv, write_json, write_svg
from .verdict import ERROR, FAKE, LOSSY, NOT_HIRES, REAL, SUSPECT, FileResult, analyze_file

AUDIO_EXTENSIONS = {
    ".flac", ".wav", ".wave", ".w64", ".rf64", ".aif", ".aiff", ".aifc", ".caf",
    ".m4a", ".mp4", ".alac", ".ape", ".wv", ".tta", ".tak", ".dsf", ".dff",
    ".mka", ".ogg", ".oga", ".opus", ".mp3", ".aac", ".wma", ".mpc", ".ac3", ".dts",
}

FILTERS = {"fake": FAKE, "suspicious": SUSPECT, "real": REAL, "not-hires": NOT_HIRES,
           "lossy": LOSSY, "error": ERROR}


def collect(paths: Iterable[str], recursive: bool) -> List[str]:
    found: List[str] = []
    for p in paths:
        if os.path.isdir(p):
            if recursive:
                for root, dirs, files in os.walk(p):
                    dirs.sort()
                    for name in sorted(files):
                        if os.path.splitext(name)[1].lower() in AUDIO_EXTENSIONS:
                            found.append(os.path.join(root, name))
            else:
                for name in sorted(os.listdir(p)):
                    fp = os.path.join(p, name)
                    if os.path.isfile(fp) and os.path.splitext(name)[1].lower() in AUDIO_EXTENSIONS:
                        found.append(fp)
        elif os.path.isfile(p):
            found.append(p)
        else:
            eprint(f"warning: not found: {p}")
    seen, unique = set(), []
    for f in found:
        a = os.path.abspath(f)
        if a not in seen:
            seen.add(a)
            unique.append(f)
    return unique


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="realhires",
        description="Scan audio files and report which are real high-resolution audio and which "
                    "are fakes (upsampled from 44.1/48 kHz, sourced from lossy files, or 16-bit "
                    "audio padded to 24-bit).",
    )
    p.add_argument("paths", nargs="+", help="audio files and/or folders to scan")
    p.add_argument("-n", "--no-recursive", action="store_true",
                   help="do not descend into sub-folders")
    p.add_argument("-j", "--jobs", type=int, default=max(1, min(4, os.cpu_count() or 1)),
                   help="files analysed in parallel (default: %(default)s)")
    p.add_argument("-t", "--max-seconds", type=float, default=None,
                   help="analyse only the first N seconds of each file (faster; default: whole file)")
    p.add_argument("-v", "--verbose", action="store_true",
                   help="show the reasoning and measurements for every file")
    p.add_argument("--only", choices=sorted(FILTERS), action="append",
                   help="only list files with this verdict (repeatable)")
    p.add_argument("--json", metavar="FILE", help="write full results as JSON")
    p.add_argument("--csv", metavar="FILE", help="write a CSV summary")
    p.add_argument("--plot", metavar="DIR", help="write an SVG spectrum plot per file into DIR")
    p.add_argument("--no-color", action="store_true", help="disable coloured output")
    p.add_argument("--version", action="version", version=f"%(prog)s {__version__}")
    return p


def main(argv=None) -> int:
    args = build_parser().parse_args(argv)
    try:
        require_tools()
    except ToolMissingError as exc:
        eprint(f"error: {exc}")
        return 2

    files = collect(args.paths, recursive=not args.no_recursive)
    if not files:
        eprint("No audio files found.")
        return 1

    color = use_color(sys.stdout, args.no_color)
    wanted = {FILTERS[o] for o in args.only} if args.only else None
    base = args.paths[0] if len(args.paths) == 1 and os.path.isdir(args.paths[0]) else None
    work = partial(analyze_file, max_seconds=args.max_seconds, keep_spectrum=bool(args.plot))

    results: List[FileResult] = []
    print(header())
    jobs = max(1, args.jobs)
    show_progress = sys.stderr.isatty()

    def handle(i: int, r: FileResult) -> None:
        if show_progress:
            eprint(f"\r\033[K", end="")
        if args.plot and r.spectrum is not None:
            write_svg(r, args.plot)
        r.spectrum = None  # free memory
        results.append(r)
        if wanted is None or r.verdict in wanted:
            print(row(r, color, base=base, verbose=args.verbose), flush=True)
        if show_progress and i < len(files):
            eprint(f"[{i}/{len(files)}] analysing…", end="", flush=True)

    if jobs == 1 or len(files) == 1:
        for i, f in enumerate(files, 1):
            handle(i, work(f))
    else:
        with ProcessPoolExecutor(max_workers=jobs) as ex:
            for i, r in enumerate(ex.map(work, files), 1):
                handle(i, r)
    if show_progress:
        eprint("\r\033[K", end="")

    print()
    print(summary(results, color))
    if args.json:
        write_json(results, args.json)
        print(f"JSON written to {args.json}")
    if args.csv:
        write_csv(results, args.csv)
        print(f"CSV written to {args.csv}")
    if args.plot:
        print(f"Spectrum plots written to {args.plot}/")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
