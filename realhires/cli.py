"""Command-line interface: `realhires PATH... [options]`."""

from __future__ import annotations

import argparse
import hashlib
import os
import re
import sys
from concurrent.futures import ThreadPoolExecutor
from functools import partial
from typing import Iterable, List, Optional

from . import __version__
from . import settings as settings_mod
from .pool import MultiProcessPool, effective_jobs
from .probe import ToolMissingError, require_tools
from .report import (eprint, header, row, setup_console, summary, summary_bars, use_color, write_csv,
                     write_json, write_svg)
from .store import ResultStore, StoreError, open_store, settings_key
from .verdict import (ERROR, FAKE, FAKE_LOSSLESS, LOSSY, NOT_HIRES, REAL, SUSPECT, FileResult,
                      analyze_file)

BACKUP_DIR_NAME = "Real-Hi-Res backups"  # same as fix.BACKUP_DIR_NAME (fix imports verdict, keep cli light)

# Kept for backwards compatibility; the effective list comes from the settings.
AUDIO_EXTENSIONS = settings_mod.Settings().extension_set()

FILTERS = {
    "fake": (FAKE, FAKE_LOSSLESS),
    "fake-hires": (FAKE,),
    "fake-lossless": (FAKE_LOSSLESS,),
    "problems": (FAKE, FAKE_LOSSLESS, SUSPECT),
    "suspicious": (SUSPECT,),
    "real": (REAL,),
    "not-hires": (NOT_HIRES,),
    "lossy": (LOSSY,),
    "error": (ERROR,),
}


def iter_audio_files(path: str, recursive: bool, extensions: Optional[set] = None):
    """Yield audio files of one folder (sorted, folder by folder) or the file itself."""
    exts = extensions or AUDIO_EXTENSIONS
    if os.path.isdir(path):
        if recursive:
            for root, dirs, files in os.walk(path):
                # never rescan originals that "fix" moved away, nor its temp files
                dirs[:] = sorted(d for d in dirs if d != BACKUP_DIR_NAME)
                for name in sorted(files):
                    if os.path.splitext(name)[1].lower() in exts and not name.startswith(".realhires-"):
                        yield os.path.join(root, name)
        else:
            for name in sorted(os.listdir(path)):
                fp = os.path.join(path, name)
                if (os.path.isfile(fp) and os.path.splitext(name)[1].lower() in exts
                        and not name.startswith(".realhires-")):
                    yield fp
    elif os.path.isfile(path):
        yield path


def collect(paths: Iterable[str], recursive: bool, extensions: Optional[set] = None) -> List[str]:
    found: List[str] = []
    for p in paths:
        if os.path.exists(p):
            found.extend(iter_audio_files(p, recursive, extensions))
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
        description="Scan audio files and report which are real high-resolution / lossless audio "
                    "and which are fakes: upsampled from 44.1/48 kHz, 16-bit padded to 24-bit, or "
                    "'fake FLAC/WAV/ALAC' files made from MP3/AAC.",
        epilog="Settings saved in the desktop app (Settings window) are used here too; see "
               f"{settings_mod.default_path()}. Use --set NAME=VALUE to override one for a run "
               "(e.g. --set cliff_db=30) and --list-settings to see all names.",
    )
    p.add_argument("paths", nargs="*", help="audio files and/or folders to scan")
    p.add_argument("-n", "--no-recursive", action="store_true",
                   help="do not descend into sub-folders")
    p.add_argument("-j", "--jobs", type=int, default=None,
                   help="files analysed in parallel; 0 = one per CPU core (default: from settings)")
    p.add_argument("-t", "--max-seconds", type=float, default=None,
                   help="analyse only the first N seconds of each file (default: from settings, "
                        "normally the whole file)")
    p.add_argument("-v", "--verbose", action="store_true",
                   help="show the full reasoning and measurements for every file")
    p.add_argument("--only", choices=sorted(FILTERS), action="append",
                   help="only list files with this verdict (repeatable; 'fake' = fake hi-res + "
                        "fake lossless, 'problems' = fake + suspicious)")
    p.add_argument("--json", metavar="FILE", help="write full results as JSON")
    p.add_argument("--csv", metavar="FILE", help="write a CSV summary")
    p.add_argument("--plot", metavar="DIR", help="write an SVG average-spectrum plot per file into DIR")
    p.add_argument("--spectrogram", metavar="DIR", help="write a PNG spectrogram per file into DIR")
    p.add_argument("--fix", action="store_true",
                   help="after scanning, fix every fake file in place so its properties show its real "
                        "resolution (e.g. a fake 24/96 becomes a genuine 16/44.1 file with the same "
                        "name, tags and cover art); originals are moved to a 'Real-Hi-Res backups' "
                        "folder")
    p.add_argument("--fix-backup", choices=["folder", "bak", "none"],
                   help="with --fix: keep originals in the backup folder (default), as <name>.bak, "
                        "or delete them")
    p.add_argument("--fix-backup-dir", metavar="DIR", help="with --fix: put backups of originals into DIR")
    p.add_argument("--fix-copy", action="store_true",
                   help="with --fix: leave the files alone and save corrected copies next to them")
    p.add_argument("--fix-dir", metavar="DIR", help="with --fix: leave the files alone and save corrected "
                                                     "copies into DIR")
    p.add_argument("--fix-replace", action="store_true", help=argparse.SUPPRESS)  # 1.4.0 name, now default
    p.add_argument("--dry-run", action="store_true", help="with --fix: only show what would be changed")
    p.add_argument("--rescan", action="store_true",
                   help="analyse every file again, even files with a saved result from an earlier scan "
                        "(the saved results are updated)")
    p.add_argument("--rescan-outdated", action="store_true",
                   help="analyse again the files whose saved result was made by older detection rules "
                        "(after an update of Real-Hi-Res)")
    p.add_argument("--no-cache", action="store_true",
                   help="neither use nor save saved results in this run")
    p.add_argument("--forget-results", action="store_true",
                   help="delete all saved results and exit")
    p.add_argument("--settings", metavar="FILE", help="use this settings file instead of the saved one")
    p.add_argument("--no-settings", action="store_true", help="ignore saved settings; use defaults")
    p.add_argument("--set", metavar="NAME=VALUE", action="append", default=[],
                   help="override one setting for this run (repeatable)")
    p.add_argument("--list-settings", action="store_true",
                   help="print every setting with its current value and exit")
    p.add_argument("--no-color", action="store_true", help="disable coloured output")
    p.add_argument("--version", action="version", version=f"%(prog)s {__version__}")
    return p


def _load_settings(args) -> settings_mod.Settings:
    cfg = settings_mod.Settings() if args.no_settings else settings_mod.load(args.settings)
    for item in args.set:
        if "=" not in item:
            raise ValueError(f"--set expects NAME=VALUE, got {item!r}")
        name, value = item.split("=", 1)
        cfg.set_value(name.strip(), value.strip())
    return cfg


def _safe_name(path: str) -> str:
    stem = re.sub(r"[^\w.-]+", "_", os.path.splitext(os.path.basename(path))[0])[:120] or "file"
    return f"{stem}_{hashlib.sha1(path.encode('utf-8', 'replace')).hexdigest()[:6]}"


def write_spectrogram_png(r: FileResult, out_dir: str, cfg: settings_mod.Settings) -> Optional[str]:
    from .imaging import spectrogram_png

    sg = (r.plot or {}).get("spectrogram")
    if not sg:
        return None
    markers = []
    if r.upsampled_from:
        markers.append((r.upsampled_from / 2.0, (0, 200, 255)))
    if r.lossy_cutoff_hz:
        markers.append((r.lossy_cutoff_hz, (255, 80, 80)))
    os.makedirs(out_dir, exist_ok=True)
    path = os.path.join(out_dir, _safe_name(r.path) + ".png")
    with open(path, "wb") as fh:
        fh.write(spectrogram_png(sg, 1000, 400, cfg.spectrogram_range_db, cfg.colormap, markers))
    return path


def main(argv=None) -> int:
    setup_console()
    args = build_parser().parse_args(argv)
    try:
        cfg = _load_settings(args)
    except ValueError as exc:
        eprint(f"error: {exc}")
        return 2
    if args.list_settings:
        for info in settings_mod.Settings.field_info():
            print(f"{info['name']:<22} = {getattr(cfg, info['name'])!r:<28} # {info['label']}")
        return 0
    if args.forget_results:
        try:
            with ResultStore() as st:
                n = st.forget()
                print(f"Deleted {n} saved result(s) from {st.path}.")
        except StoreError as exc:
            eprint(f"error: {exc}")
            return 2
        return 0
    if not args.paths:
        build_parser().error("no files or folders given")
    try:
        require_tools()
    except ToolMissingError as exc:
        eprint(f"error: {exc}")
        return 2

    recursive = cfg.recursive and not args.no_recursive
    files = collect(args.paths, recursive=recursive, extensions=cfg.extension_set())
    if not files:
        eprint("No audio files found.")
        return 1

    color = use_color(sys.stdout, args.no_color)
    wanted = {v for o in args.only for v in FILTERS[o]} if args.only else None
    base = args.paths[0] if len(args.paths) == 1 and os.path.isdir(args.paths[0]) else None
    max_seconds = args.max_seconds if args.max_seconds is not None else (cfg.max_seconds or None)
    work = partial(analyze_file, max_seconds=max_seconds, keep_spectrum=bool(args.plot),
                   keep_plot=bool(args.spectrogram), settings=cfg)

    if args.fix_replace:
        cfg.fix_output = "Replace the file"
    if args.fix_copy:
        cfg.fix_output = "Saving a copy next to it"
    if args.fix_dir:
        cfg.fix_output, cfg.fix_folder = "Saving a copy into a folder", os.path.abspath(args.fix_dir)
    if args.fix_backup:
        cfg.fix_backup = {"folder": "Move the original to a backup folder",
                          "bak": "Keep it as <name>.bak next to the file",
                          "none": "Delete it (no backup)"}[args.fix_backup]
    if args.fix_backup_dir:
        cfg.fix_backup_folder = os.path.abspath(args.fix_backup_dir)

    # Saved results of files that have not changed since an earlier scan are used
    # as they are; only the other files are analysed (and then saved).
    store = None if args.no_cache else open_store(cfg)
    skey = settings_key(cfg)
    ready = {}
    if store is not None and not args.rescan and not args.plot:  # SVG plots need the full spectrum
        again_outdated = args.rescan_outdated or cfg.rescan_outdated
        for i, f in enumerate(files):
            r = store.lookup(f, skey, max_seconds)
            if r is None or (r.saved["outdated"] and again_outdated):
                continue
            if args.spectrogram:
                r.plot = store.get_plot(r.path, skey)
                if not (r.plot or {}).get("spectrogram"):
                    continue  # no saved spectrogram: analyse to draw one
            ready[i] = r

    results: List[FileResult] = []
    print(header())
    jobs = effective_jobs(args.jobs if args.jobs is not None else cfg.jobs)
    show_progress = sys.stderr.isatty()

    def save(r: FileResult) -> FileResult:
        if store is not None and r.saved is None:
            store.put(r, skey, max_seconds, plot=r.plot)
        return r

    def handle(i: int, r: FileResult) -> None:
        if show_progress:
            eprint("\r\033[K", end="")
        if args.plot and r.spectrum is not None:
            write_svg(r, args.plot)
        if args.spectrogram:
            write_spectrogram_png(r, args.spectrogram, cfg)
        r.spectrum = None  # free memory
        r.plot = None
        results.append(r)
        if wanted is None or r.verdict in wanted:
            print(row(r, color, base=base, verbose=args.verbose), flush=True)
        if show_progress and i < len(files):
            eprint(f"[{i}/{len(files)}] analysing…", end="", flush=True)

    todo = [i for i in range(len(files)) if i not in ready]
    if jobs == 1 or len(todo) <= 1:
        for i, f in enumerate(files):
            handle(i + 1, ready[i] if i in ready else save(work(f)))
    else:
        with MultiProcessPool(min(jobs, len(todo))) as ex:
            futures = {i: ex.submit(work, files[i]) for i in todo}
            for i in range(len(files)):
                handle(i + 1, ready[i] if i in ready else save(futures[i].result()))
    if show_progress:
        eprint("\r\033[K", end="")

    print()
    print(summary(results, color))
    print(summary_bars(results, color))
    if store is not None:
        reused = [r for r in results if r.saved]
        print(f"Saved results: {len(reused)} file(s) unchanged since an earlier scan were not analysed "
              f"again, {len(results) - len(reused)} analysed. (--rescan analyses everything again.)")
        old = sum(1 for r in reused if r.saved["outdated"])
        if old:
            print(f"{old} of them were checked by older detection rules; --rescan-outdated analyses them "
                  "with the current rules.")
    if args.json:
        write_json(results, args.json)
        print(f"JSON written to {args.json}")
    if args.csv:
        write_csv(results, args.csv)
        print(f"CSV written to {args.csv}")
    if args.plot:
        print(f"Spectrum plots written to {args.plot}/")
    if args.spectrogram:
        print(f"Spectrograms written to {args.spectrogram}/")
    try:
        if args.fix:
            return run_fixes(results, cfg, jobs, args.dry_run, store=store)
        return 0
    finally:
        if store is not None:
            store.close()


def run_fixes(results: List[FileResult], cfg, jobs: int, dry_run: bool = False,
              store: Optional[ResultStore] = None) -> int:
    from .fix import plan_fix, prepare_gapless, run_fix

    plans = []
    for r in results:
        plan, why = plan_fix(r, cfg)
        if plan:
            plans.append(plan)
    print()
    if not plans:
        print("Fix: nothing to convert (no fake hi-res / fake lossless files that can be reduced).")
        return 0
    action = "fix in place" if plans[0].replace_original else "save corrected copies of"
    print(f"Fix: {'would ' if dry_run else 'going to '}{action} {len(plans)} file(s):")
    if dry_run:
        for p in plans:
            dest = "(same file)" if p.replace_original and p.output == p.source else p.output
            print(f"  {p.description:<22} {p.source}\n  {'':<22} -> {dest}")
            if p.replace_original:
                print(f"  {'':<22}    original: " + {"folder": f"moved to {p.backup_dir}",
                                                      "bak": "kept as .bak", "none": "deleted"}[p.backup])
        return 0
    # Read the joins between album neighbours before any file is replaced.
    joins = prepare_gapless(plans, results, progress=lambda m: print(f"Fix: {m}", flush=True))
    if joins:
        print(f"Fix: {joins} gapless join(s) found; those tracks are converted together with their "
              "neighbours so the joins stay seamless.")
    failed = 0
    with ThreadPoolExecutor(max_workers=max(1, min(jobs, 16))) as ex:
        for out in ex.map(lambda p: run_fix(p, cfg), plans):
            mark = "OK  " if out.ok else "FAIL"
            failed += not out.ok
            if store is not None and out.new_result is not None:
                store.put(out.new_result, settings_key(cfg))  # the fixed file is known from now on
            print(f"  {mark} {out.plan.source}\n       {out.message}")
    print(f"Fix: {len(plans) - failed} fixed, {failed} failed.")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
