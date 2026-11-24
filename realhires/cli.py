"""Command-line interface: `realhires PATH... [options]`."""

from __future__ import annotations

import argparse
import hashlib
import os
import re
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from functools import partial
from typing import Iterable, List, Optional

from . import __version__
from . import settings as settings_mod
from .pool import MultiProcessPool, effective_jobs
from .probe import ToolMissingError, external_env, require_tools
from .report import (eprint, header, row, setup_console, summary, summary_bars, use_color, write_csv,
                     write_json, write_svg)
from .eta import ScanEta, file_sizes
from .lyrics import EXISTING_CLI
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
    p.add_argument("--dry-run", action="store_true",
                   help="with --fix or --lyrics: only show what would be changed (lyrics are still looked "
                        "up, but nothing is saved)")
    p.add_argument("--lyrics", action="store_true",
                   help="after scanning, download synced lyrics from LRCLIB (lrclib.net) for every scanned song "
                        "and save them as <name>.lrc next to it (plain lyrics as <name>.txt), as LRCGET does; "
                        "songs are looked up by the title, artist, album and length in their tags")
    p.add_argument("--lyrics-only", action="store_true",
                   help="download lyrics as --lyrics does, without analysing the files (only their tags are "
                        "read)")
    p.add_argument("--lyrics-existing", choices=list(EXISTING_CLI),
                   help="with --lyrics: skip songs that have an .lrc (skip-synced, the default), skip songs "
                        "that have an .lrc or .txt (skip-any), or download again and replace (replace)")
    p.add_argument("--check-lyrics-server", action="store_true",
                   help="check that the LRCLIB server can be reached (one request) and exit")
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


DOUBLE_CLICK_MESSAGE = (
    "This is the command-line tool of Real-Hi-Res. It checks the folders or files you name, for example\n"
    "  {cli} \"D:\\Music\"\n"
    "in a terminal. The desktop app is {gui} in the same folder.\n\nPress Enter to close this window.")


def started_from_explorer() -> bool:
    """Windows: True when this program has a console window of its own, which is what
    happens when it is double-clicked in Explorer rather than started from a terminal."""
    if os.name != "nt":
        return False
    try:
        import ctypes

        ids = (ctypes.c_uint32 * 4)()
        return ctypes.windll.kernel32.GetConsoleProcessList(ids, 4) == 1
    except Exception:  # noqa: BLE001 - no console API: not double-clicked
        return False


def open_app_instead(argv, gui_name: str = "RealHiRes.exe", cli_name: str = "realhires-cli.exe",
                     started=started_from_explorer, launch=None, wait=input) -> Optional[int]:
    """A double-clicked command-line tool (no arguments, its own console) opens the desktop app
    next to it instead of printing a usage error into a window that closes at once.  Returns
    the exit code, or None when the command line should run as usual."""
    if argv or not started():
        return None
    gui = os.path.join(os.path.dirname(os.path.abspath(sys.executable if getattr(sys, "frozen", False)
                                                       else sys.argv[0])), gui_name)
    if os.path.isfile(gui):
        try:
            # PYINSTALLER_RESET_ENVIRONMENT: the app is a program of its own, not a child of this one
            (launch or subprocess.Popen)([gui], env=dict(external_env(), PYINSTALLER_RESET_ENVIRONMENT="1"))
            return 0
        except OSError:
            pass
    print(DOUBLE_CLICK_MESSAGE.format(cli=cli_name, gui=gui_name), flush=True)
    try:
        wait()
    except (EOFError, KeyboardInterrupt):
        pass
    return 2


def main(argv=None) -> int:
    setup_console()
    if argv is None:
        opened = open_app_instead(sys.argv[1:])
        if opened is not None:
            return opened
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
    if args.check_lyrics_server:
        from .lyrics import LyricsError, check_server

        try:
            print(check_server(cfg.lyrics_server))
        except LyricsError as exc:
            eprint(f"error: {exc}")
            return 1
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
    if args.lyrics_existing:
        cfg.lyrics_existing = EXISTING_CLI[args.lyrics_existing]
    if args.lyrics_only:
        return run_lyrics_cli(files, cfg, dry_run=args.dry_run, verbose=args.verbose)

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
        if eta is not None and r.saved is None:
            eta.add(r.path)
        results.append(r)
        if wanted is None or r.verdict in wanted:
            print(row(r, color, base=base, verbose=args.verbose, cfg=cfg), flush=True)
        if show_progress and i < len(files):
            eprint(f"[{i}/{len(files)}] analysing… {eta.text() if eta else ''}", end="", flush=True)

    todo = [i for i in range(len(files)) if i not in ready]
    eta = ScanEta(file_sizes(os.path.abspath(files[i]) for i in todo)) if show_progress else None
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
        rc = run_fixes(results, cfg, jobs, args.dry_run, store=store) if args.fix else 0
        if args.lyrics:
            rc = max(rc, run_lyrics_cli([r.path for r in results], cfg, args.dry_run, args.verbose,
                                        {r.path: r for r in results}))
        return rc
    finally:
        if store is not None:
            store.close()


def run_fixes(results: List[FileResult], cfg, jobs: int, dry_run: bool = False,
              store: Optional[ResultStore] = None) -> int:
    from .fix import plan_fixes, prepare_gapless, run_fix, skip_summary

    plans, skipped = plan_fixes(results, cfg)
    print()
    if skipped:
        print(f"Fix: {skip_summary(skipped)}")
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


def run_lyrics_cli(paths: List[str], cfg, dry_run: bool = False, verbose: bool = False,
                   results: Optional[dict] = None) -> int:
    """Download synced lyrics for `paths` (--lyrics, --lyrics-only), one song at a time."""
    from .lyrics import FAILED, SKIPPED, STATUS_LABEL, LrclibClient, lyrics_summary, run_lyrics, skip_reason

    client = LrclibClient(cfg.lyrics_server)
    todo = [p for p in paths if not skip_reason(p, cfg)]
    print()
    skipped = len(paths) - len(todo)
    note = (f"; {skipped} already {'has' if skipped == 1 else 'have'} lyrics and {'is' if skipped == 1 else 'are'} "
            "skipped" if skipped else "")
    print(f"Lyrics: looking up {len(todo)} song(s) on LRCLIB ({client.server}), one at a time{note}"
          + (" (dry run: nothing is saved)." if dry_run else "."), flush=True)
    show_progress = sys.stderr.isatty()
    eta = ScanEta({p: 1 for p in todo}) if show_progress else None
    outcomes = []
    try:
        for o in run_lyrics(paths, cfg, results, dry_run, client=client):
            outcomes.append(o)
            if show_progress:
                eprint("\r\033[K", end="")
            if eta is not None:
                eta.add(o.path)
            if o.status != SKIPPED or verbose:
                name = o.track.label if o.track else os.path.basename(o.path)
                print(f"  {STATUS_LABEL[o.status]:<20} {name}\n  {'':<20} {o.message}", flush=True)
            if show_progress and len(outcomes) < len(paths):
                eprint(f"[{len(outcomes)}/{len(paths)}] looking up lyrics… {eta.text() if eta else ''}",
                       end="", flush=True)
    except KeyboardInterrupt:
        if show_progress:
            eprint("\r\033[K", end="")
        print(f"Lyrics: stopped after {len(outcomes)} of {len(paths)} song(s): {lyrics_summary(outcomes)}")
        return 130
    if show_progress:
        eprint("\r\033[K", end="")
    print(f"Lyrics: {lyrics_summary(outcomes)}")
    return 1 if any(o.status == FAILED for o in outcomes) else 0


if __name__ == "__main__":
    raise SystemExit(main())
