"""Saved scan results, so that files which have not changed are never analysed
again: not on the next scan, not after restarting, and not after updating
Real-Hi-Res.

The results live in one SQLite database in the user's data folder, outside
the program folder, so a new version finds them where the old one left them:

* Windows: %LOCALAPPDATA%\\RealHiRes\\results.sqlite
* macOS:   ~/Library/Application Support/RealHiRes/results.sqlite
* Linux:   $XDG_DATA_HOME/realhires/results.sqlite (default ~/.local/share/realhires)

$REALHIRES_RESULTS overrides the path (the tests use it).

A saved result is reused when the file still has the same size and
modification time, it was made with the same detection settings
(`settings_key()`), and it covers at least as much of the file as asked for.
A file that was moved or renamed is recognised by its size, its modification
time (which moving keeps) and a fingerprint of its first and last 64 KB.  Results made by an older version of the detection rules
(`verdict.DETECTOR_VERSION`) are still returned, marked as outdated in
`FileResult.saved`, so the caller decides whether to analyse the file again.
Errors are never saved, because they are often temporary.

Each row can also hold the file's graphs (the compact spectrum and the
spectrogram, about 30-90 KB), within a size limit after which the least
recently viewed graphs are dropped; the result itself (about 1.5 KB) is kept.
Nothing is pickled: results are JSON and the spectrogram is the zlib-compressed
byte matrix that analysis.spectrogram_result() makes.

A connection belongs to the thread that opened it, so every thread that needs
the store opens its own `ResultStore`.  SQLite's write-ahead log lets the GUI's
threads and a CLI run use the database at the same time.
"""

from __future__ import annotations

import hashlib
import json
import os
import sqlite3
import sys
import time
import zlib
from dataclasses import fields
from typing import Optional

from . import __version__
from .verdict import DETECTOR_VERSION, ERROR, FileResult

SCHEMA_VERSION = 1
DB_NAME = "results.sqlite"
DEFAULT_GRAPHS_MB = 500
_INT_KEYS = ("cliffs_db", "deep_cliffs_db", "bit_usage")   # dicts keyed by numbers
_NOT_SAVED = ("spectrum", "plot", "saved")
_TRIM_EVERY = 50   # graph writes between checks of the size limit
_FP_BYTES = 64 * 1024


class StoreError(Exception):
    """The saved results cannot be used (the caller carries on without them)."""


def data_dir() -> str:
    if sys.platform.startswith("win"):
        base = os.environ.get("LOCALAPPDATA") or os.path.expanduser("~\\AppData\\Local")
        return os.path.join(base, "RealHiRes")
    if sys.platform == "darwin":
        return os.path.expanduser("~/Library/Application Support/RealHiRes")
    base = os.environ.get("XDG_DATA_HOME") or os.path.expanduser("~/.local/share")
    return os.path.join(base, "realhires")


def default_path() -> str:
    return os.environ.get("REALHIRES_RESULTS") or os.path.join(data_dir(), DB_NAME)


def settings_key(cfg=None) -> str:
    """Fingerprint of the settings that can change a result (the Detection
    group).  Display, scanning and fixing settings do not change results."""
    from .settings import Settings

    cfg = cfg or Settings()
    values = {f["name"]: getattr(cfg, f["name"]) for f in Settings.field_info() if f["group"] == "Detection"}
    return hashlib.sha1(json.dumps(values, sort_keys=True).encode()).hexdigest()[:16]


# ------------------------------------------------------------ (de)coding ---
def result_to_json(r: FileResult) -> str:
    d = {f.name: getattr(r, f.name) for f in fields(r) if f.name not in _NOT_SAVED}
    return json.dumps(d, separators=(",", ":"))  # NaN and ±Infinity round-trip


def result_from_json(text: str, path: str) -> FileResult:
    d = json.loads(text)
    known = {f.name for f in fields(FileResult)}
    d = {k: v for k, v in d.items() if k in known and k not in _NOT_SAVED}
    for k in _INT_KEYS:
        if isinstance(d.get(k), dict):
            d[k] = {int(key): v for key, v in d[k].items()}
    d["path"] = path
    return FileResult(**d)


def plot_to_blobs(plot: dict):
    p = dict(plot)
    sg = p.pop("spectrogram", None)
    p["spectrogram_meta"] = ({k: (list(v) if k == "shape" else v) for k, v in sg.items() if k != "data"}
                             if sg else None)
    return zlib.compress(json.dumps(p).encode(), 6), (bytes(sg["data"]) if sg else None)


def plot_from_blobs(graphs: bytes, spectro: Optional[bytes]) -> dict:
    p = json.loads(zlib.decompress(graphs))
    meta = p.pop("spectrogram_meta", None)
    if meta and spectro is not None:
        p["spectrogram"] = dict(meta, data=bytes(spectro), shape=tuple(meta["shape"]))
    return p


def fingerprint(path: str, size: int) -> str:
    """Hash of the size and the first and last 64 KB: tells a moved or renamed
    file from a different one with the same size and modification time (files
    unpacked from one archive share their times; WAV tracks of the same length
    share their size)."""
    h = hashlib.sha1(str(size).encode())
    with open(path, "rb") as f:
        h.update(f.read(_FP_BYTES))
        if size > 2 * _FP_BYTES:
            f.seek(size - _FP_BYTES)
            h.update(f.read(_FP_BYTES))
    return h.hexdigest()


def covers(whole: bool, limit_s: float, wanted: Optional[float]) -> bool:
    """Does a result made with `limit_s` (0 = whole file) answer a request
    for the first `wanted` seconds (None or 0 = the whole file)?"""
    return bool(whole) or (bool(wanted) and limit_s >= wanted)


# ----------------------------------------------------------------- store ---
_COLUMNS = ("path, skey, fp, size, mtime_ns, limit_s, whole, detector, app_version, saved, used, "
            "result, graphs, spectro, gbytes")


class ResultStore:
    def __init__(self, path: Optional[str] = None, graphs_mb: float = DEFAULT_GRAPHS_MB):
        self.path = os.path.abspath(path or default_path())
        self.graphs_limit = max(0.0, float(graphs_mb)) * 1e6
        self._graph_writes = 0
        try:
            os.makedirs(os.path.dirname(self.path), exist_ok=True)
            self.db = self._open()
        except (OSError, sqlite3.Error) as exc:
            raise StoreError(f"cannot open the saved results in {self.path}: {exc}") from exc

    def _open(self) -> sqlite3.Connection:
        try:
            db = self._connect()
        except sqlite3.DatabaseError:
            # A damaged file (a crash on a full disk, a sync tool...) must not stop
            # scanning: keep it aside and start a fresh one.
            os.replace(self.path, f"{self.path}.damaged-{int(time.time())}")
            db = self._connect()
        return db

    def _connect(self) -> sqlite3.Connection:
        db = sqlite3.connect(self.path, timeout=30, isolation_level=None)
        try:
            return self._setup(db)
        except BaseException:
            db.close()  # Windows cannot rename the damaged file while it is open
            raise

    def _setup(self, db: sqlite3.Connection) -> sqlite3.Connection:
        db.row_factory = sqlite3.Row
        db.execute("PRAGMA auto_vacuum=INCREMENTAL")  # only takes effect on a new file
        try:
            db.execute("PRAGMA journal_mode=WAL")
        except sqlite3.OperationalError:
            pass  # e.g. a network drive: the default journal works too
        db.execute("PRAGMA synchronous=NORMAL")
        db.execute("CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT)")
        row = db.execute("SELECT value FROM meta WHERE key='schema'").fetchone()
        if row and int(row[0]) > SCHEMA_VERSION:
            raise StoreError(f"{self.path} was written by a newer Real-Hi-Res; it is left untouched")
        db.execute("""CREATE TABLE IF NOT EXISTS results (
            path TEXT NOT NULL, skey TEXT NOT NULL, fp TEXT NOT NULL,
            size INTEGER NOT NULL, mtime_ns INTEGER NOT NULL,
            limit_s REAL NOT NULL, whole INTEGER NOT NULL,
            detector INTEGER NOT NULL, app_version TEXT NOT NULL,
            saved REAL NOT NULL, used REAL NOT NULL,
            result TEXT NOT NULL, graphs BLOB, spectro BLOB, gbytes INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (path, skey))""")
        db.execute("CREATE INDEX IF NOT EXISTS results_moved ON results (size, mtime_ns)")
        db.execute("INSERT OR REPLACE INTO meta VALUES ('schema', ?)", (str(SCHEMA_VERSION),))
        db.execute("SELECT count(*) FROM results").fetchone()  # fails here on a damaged file
        return db

    # ---- reading ---------------------------------------------------------
    def lookup(self, path: str, skey: str, wanted: Optional[float] = None) -> Optional[FileResult]:
        """The saved result for this file as it is now, or None.  The result's
        `saved` dict says when and by which version it was made, whether that
        version's detection rules are older than this one's ("outdated") and
        whether graphs are saved with it."""
        path = os.path.abspath(path)
        try:
            st = os.stat(path)
        except OSError:
            return None
        try:
            row = self.db.execute(f"SELECT {_COLUMNS} FROM results WHERE path=? AND skey=?",
                                  (path, skey)).fetchone()
            moved_from = None
            if not (row and row["size"] == st.st_size and row["mtime_ns"] == st.st_mtime_ns
                    and covers(row["whole"], row["limit_s"], wanted)):
                row, fp = None, None
                for cand in self.db.execute(
                        f"SELECT {_COLUMNS} FROM results WHERE size=? AND mtime_ns=? AND skey=? AND path!=? "
                        "ORDER BY saved DESC", (st.st_size, st.st_mtime_ns, skey, path)).fetchall():
                    if not covers(cand["whole"], cand["limit_s"], wanted):
                        continue
                    fp = fp or fingerprint(path, st.st_size)
                    if cand["fp"] == fp:
                        row, moved_from = cand, cand["path"]
                        break
            if row is None:
                return None
            r = result_from_json(row["result"], path)
            if moved_from:  # remember it under its new name too
                self.db.execute(f"INSERT OR REPLACE INTO results ({_COLUMNS}) VALUES "
                                "(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                                (path, *tuple(row)[1:]))
        except (sqlite3.Error, OSError, ValueError, TypeError) as exc:
            print(f"Real-Hi-Res: ignoring a saved result for {path}: {exc}", file=sys.stderr)
            return None
        r.saved = {"time": row["saved"], "version": row["app_version"], "detector": row["detector"],
                   "outdated": row["detector"] < DETECTOR_VERSION, "graphs": row["graphs"] is not None,
                   "moved_from": moved_from}
        return r

    def get_plot(self, path: str, skey: str) -> Optional[dict]:
        try:
            row = self.db.execute("SELECT graphs, spectro FROM results WHERE path=? AND skey=?",
                                  (os.path.abspath(path), skey)).fetchone()
            if not row or row["graphs"] is None:
                return None
            self.db.execute("UPDATE results SET used=? WHERE path=? AND skey=?",
                            (time.time(), os.path.abspath(path), skey))
            return plot_from_blobs(row["graphs"], row["spectro"])
        except (sqlite3.Error, zlib.error, ValueError) as exc:
            print(f"Real-Hi-Res: cannot read saved graphs for {path}: {exc}", file=sys.stderr)
            return None

    # ---- writing ---------------------------------------------------------
    def put(self, r: FileResult, skey: str, wanted: Optional[float] = None, plot: Optional[dict] = None) -> bool:
        """Save one result (and its graphs, if given).  Returns False when it
        was not saved: errors, files that are gone, or a database problem."""
        if r.verdict == ERROR:
            return False
        path = os.path.abspath(r.path)
        try:
            st = os.stat(path)
            fp = fingerprint(path, st.st_size)
        except OSError:
            return False
        whole = (not wanted) or bool(r.duration and r.analysed_seconds >= r.duration - 0.5)
        now = time.time()
        graphs = spectro = None
        if plot and self.graphs_limit > 0:
            graphs, spectro = plot_to_blobs(plot)
        try:
            self.db.execute("BEGIN IMMEDIATE")
            if graphs is None:  # keep graphs saved earlier for this very file
                old = self.db.execute("SELECT graphs, spectro FROM results WHERE path=? AND skey=? AND size=? "
                                      "AND mtime_ns=?", (path, skey, st.st_size, st.st_mtime_ns)).fetchone()
                if old:
                    graphs, spectro = old["graphs"], old["spectro"]
            # the file changed: results saved under other settings are stale too
            self.db.execute("DELETE FROM results WHERE path=? AND (size!=? OR mtime_ns!=?)",
                            (path, st.st_size, st.st_mtime_ns))
            gbytes = len(graphs or b"") + len(spectro or b"")
            self.db.execute(f"INSERT OR REPLACE INTO results ({_COLUMNS}) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                            (path, skey, fp, st.st_size, st.st_mtime_ns,
                             float(wanted or 0), int(whole), DETECTOR_VERSION, __version__, now, now,
                             result_to_json(r), graphs, spectro, gbytes))
            self.db.execute("COMMIT")
        except sqlite3.Error as exc:
            self._rollback()
            print(f"Real-Hi-Res: cannot save the result for {path}: {exc}", file=sys.stderr)
            return False
        if plot:
            self._graph_writes += 1
            if self._graph_writes % _TRIM_EVERY == 0:
                self.trim_graphs()
        return True

    def put_plot(self, path: str, skey: str, plot: dict) -> None:
        """Add graphs to a saved result (they were drawn later, on demand)."""
        if not plot or self.graphs_limit <= 0:
            return
        graphs, spectro = plot_to_blobs(plot)
        try:
            self.db.execute("UPDATE results SET graphs=?, spectro=?, gbytes=?, used=? WHERE path=? AND skey=?",
                            (graphs, spectro, len(graphs) + len(spectro or b""), time.time(),
                             os.path.abspath(path), skey))
        except sqlite3.Error as exc:
            print(f"Real-Hi-Res: cannot save graphs for {path}: {exc}", file=sys.stderr)

    def trim_graphs(self) -> None:
        """Drop the least recently viewed graphs above the size limit (the
        results themselves stay)."""
        try:
            total = self.db.execute("SELECT COALESCE(SUM(gbytes), 0) FROM results").fetchone()[0]
            if total <= self.graphs_limit:
                return
            target = 0.9 * self.graphs_limit
            drop = []
            for row in self.db.execute("SELECT rowid, gbytes FROM results WHERE gbytes > 0 ORDER BY used"):
                if total <= target:
                    break
                drop.append(row[0])
                total -= row[1]
            self.db.execute("BEGIN IMMEDIATE")
            self.db.executemany("UPDATE results SET graphs=NULL, spectro=NULL, gbytes=0 WHERE rowid=?",
                                [(i,) for i in drop])
            self.db.execute("COMMIT")
            self.db.execute("PRAGMA incremental_vacuum")
        except sqlite3.Error as exc:
            self._rollback()
            print(f"Real-Hi-Res: cannot trim saved graphs: {exc}", file=sys.stderr)

    def forget(self, paths=None) -> int:
        """Delete the saved results of these files (all of them when None)."""
        try:
            if paths is None:
                n = self.db.execute("SELECT count(*) FROM results").fetchone()[0]
                self.db.execute("DELETE FROM results")
                self.db.execute("VACUUM")
                return n
            self.db.execute("BEGIN IMMEDIATE")
            n = 0
            for p in paths:
                n += self.db.execute("DELETE FROM results WHERE path=?", (os.path.abspath(p),)).rowcount
            self.db.execute("COMMIT")
            return n
        except sqlite3.Error as exc:
            self._rollback()
            raise StoreError(str(exc)) from exc

    def stats(self) -> dict:
        row = self.db.execute("SELECT count(*), COALESCE(SUM(gbytes), 0), COUNT(DISTINCT path) "
                              "FROM results").fetchone()
        try:
            size = sum(os.path.getsize(self.path + ext) for ext in ("", "-wal") if os.path.exists(self.path + ext))
        except OSError:
            size = 0
        return {"results": row[0], "files": row[2], "graph_bytes": row[1], "disk_bytes": size, "path": self.path}

    def _rollback(self) -> None:
        try:
            self.db.execute("ROLLBACK")
        except sqlite3.Error:
            pass

    def close(self) -> None:
        try:
            self.trim_graphs()
            self.db.close()
        except sqlite3.Error:
            pass

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()


def open_store(cfg=None, path: Optional[str] = None) -> Optional[ResultStore]:
    """The store as the settings ask for it, or None when saving results is
    turned off or the database cannot be used (scanning works without it)."""
    if cfg is not None and not getattr(cfg, "save_results", True):
        return None
    try:
        return ResultStore(path, graphs_mb=getattr(cfg, "saved_graphs_mb", DEFAULT_GRAPHS_MB))
    except StoreError as exc:
        print(f"Real-Hi-Res: {exc}; results are not saved this time.", file=sys.stderr)
        return None
