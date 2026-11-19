"""Desktop GUI (Tkinter, part of the Python standard library).

* Pick folders or files and scan them in parallel.
* Scroll through the results.  By default only problem files are listed:
  FAKE LOSSLESS ("fake FLAC", made from MP3/AAC), FAKE HI-RES (upsampled or
  bit-padded) and SUSPICIOUS.  The filter checkboxes show the rest.
* The dashboard below the list shows the selected file's verdict, a one-line
  reason, key facts, its spectrogram and average spectrum, and buttons to
  open the file or its folder.
* After every scan a summary window shows what percentage of files got each
  verdict.
* Settings (detection thresholds, scanning, display) are edited in the
  Settings window and saved for the next start (also used by the CLI).

Run with ``realhires-gui`` (installed), ``python -m realhires.gui``, or the
``RealHiRes`` executable from a release package.
"""

from __future__ import annotations

import multiprocessing
import os
import pickle
import queue
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from concurrent.futures import ProcessPoolExecutor, as_completed
from typing import Dict, List, Optional

try:
    import tkinter as tk
    from tkinter import filedialog, messagebox, ttk
except ImportError as exc:  # e.g. Debian/Ubuntu Python without python3-tk
    raise SystemExit(
        "The Real-Hi-Res GUI needs Tkinter, which this Python does not have.\n"
        "Install it (Debian/Ubuntu: sudo apt install python3-tk; Fedora: sudo dnf install "
        "python3-tkinter; Homebrew: brew install python-tk), use a release package, or use "
        "the command-line tool `realhires` instead.\n"
        f"(Details: {exc})"
    ) from exc

from . import __version__
from . import settings as settings_mod
from .cli import collect
from .probe import ToolMissingError, external_env, require_tools
from .report import write_csv, write_json
from .verdict import (ERROR, FAKE, FAKE_LOSSLESS, LOSSY, NOT_HIRES, PROBLEM_VERDICTS, REAL, SUSPECT,
                      VERDICT_ORDER, FileResult, analyze_file, verdict_breakdown)

VERDICT_STYLE = {
    FAKE_LOSSLESS: ("Fake lossless", "#a3122a", "Lossless file (FLAC/WAV/ALAC...) made from an MP3/AAC"),
    FAKE: ("Fake hi-res", "#d32f2f", "Upsampled and/or 16-bit padded"),
    SUSPECT: ("Suspicious", "#b26a00", "Could not be confirmed either way"),
    REAL: ("Real hi-res", "#2e7d32", "Genuine high resolution"),
    NOT_HIRES: ("CD quality", "#1565c0", "Genuine lossless, not hi-res"),
    LOSSY: ("Lossy format", "#6a1b9a", "MP3/AAC/... file"),
    ERROR: ("Error", "#616161", "Could not be analysed"),
}
PENDING = "PENDING"  # GUI-only state of a listed file that has not been scanned yet
PENDING_STYLE = ("Not scanned", "#808080")


def _work(path: str, max_seconds: Optional[float], cfg) -> FileResult:
    """Runs in a worker process.  Must be module-level so it can be pickled."""
    return analyze_file(path, max_seconds=max_seconds, keep_plot=True, settings=cfg)


# ---------------------------------------------------------------- helpers ---
def reveal_in_file_manager(path: str) -> None:
    env = external_env()  # do not leak the frozen app's library path
    if sys.platform.startswith("win"):
        subprocess.Popen(["explorer", "/select,", os.path.normpath(path)], env=env)
    elif sys.platform == "darwin":
        subprocess.Popen(["open", "-R", path], env=env)
    else:
        subprocess.Popen(["xdg-open", os.path.dirname(path) or "."], env=env)


def open_with_default_app(path: str) -> None:
    if sys.platform.startswith("win"):
        os.startfile(path)  # type: ignore[attr-defined]
    elif sys.platform == "darwin":
        subprocess.Popen(["open", path], env=external_env())
    else:
        subprocess.Popen(["xdg-open", path], env=external_env())


def fmt_duration(sec: Optional[float]) -> str:
    if sec is None:
        return "?"
    m, s = divmod(int(round(sec)), 60)
    h, m = divmod(m, 60)
    return f"{h}:{m:02d}:{s:02d}" if h else f"{m}:{s:02d}"


def list_source(source: str, cfg, out: queue.Queue) -> None:
    """Find the audio files of one added folder/file (background thread) and
    post ("listed", source, [absolute paths]) so they can be shown at once."""
    try:
        files = [os.path.abspath(f) for f in collect([source], recursive=cfg.recursive,
                                                     extensions=cfg.extension_set())]
    except OSError:
        files = []
    out.put(("listed", source, files))


class Scanner(threading.Thread):
    """Background thread: fans the listed files out to worker processes and
    posts ("total"|"result"|"done"|"error", payload) messages to a queue."""

    def __init__(self, files: List[str], cfg, max_seconds: Optional[float], out: queue.Queue):
        super().__init__(daemon=True)
        self.files, self.cfg, self.max_seconds, self.out = files, cfg, max_seconds, out
        self.stop_event = threading.Event()

    def run(self) -> None:
        try:
            files = list(self.files)
            self.out.put(("total", len(files)))
            if not files:
                self.out.put(("done", None))
                return
            ctx = multiprocessing.get_context("spawn")  # safe with a GUI thread running
            with ProcessPoolExecutor(max_workers=max(1, int(self.cfg.jobs)), mp_context=ctx) as ex:
                futures = {ex.submit(_work, f, self.max_seconds, self.cfg): f for f in files}
                for fut in as_completed(futures):
                    if self.stop_event.is_set():
                        for other in futures:
                            other.cancel()
                        break
                    try:
                        self.out.put(("result", fut.result()))
                    except Exception as exc:  # noqa: BLE001 - e.g. a crashed worker
                        self.out.put(("result", FileResult(
                            path=os.path.abspath(futures[fut]), verdict=ERROR, reasons=[str(exc)],
                            summary=f"Could not analyse: {exc}")))
            self.out.put(("done", "stopped" if self.stop_event.is_set() else None))
        except Exception as exc:  # noqa: BLE001
            self.out.put(("error", str(exc)))


class PlotCache:
    """Keeps each file's spectrum + spectrogram on disk instead of in memory,
    so libraries with thousands of files do not fill the RAM."""

    def __init__(self):
        self.dir = tempfile.mkdtemp(prefix="realhires-")
        self.index: Dict[str, str] = {}

    def put(self, path: str, plot: Optional[dict]) -> None:
        if not plot:
            return
        fn = os.path.join(self.dir, f"{len(self.index)}.pkl")
        with open(fn, "wb") as fh:
            pickle.dump(plot, fh, protocol=pickle.HIGHEST_PROTOCOL)
        self.index[path] = fn

    def get(self, path: str) -> Optional[dict]:
        fn = self.index.get(path)
        if not fn:
            return None
        try:
            with open(fn, "rb") as fh:
                return pickle.load(fh)  # our own data, written above
        except (OSError, pickle.PickleError, EOFError):
            return None

    def clear(self) -> None:
        shutil.rmtree(self.dir, ignore_errors=True)
        self.dir = tempfile.mkdtemp(prefix="realhires-")
        self.index.clear()

    def close(self) -> None:
        shutil.rmtree(self.dir, ignore_errors=True)


# ------------------------------------------------------------------ plots ---
class _PlotBase(tk.Canvas):
    L, R, T, B = 52, 12, 24, 28

    def __init__(self, master, **kw):
        super().__init__(master, background="#ffffff", highlightthickness=0, **kw)
        self.result: Optional[FileResult] = None
        self.plot: Optional[dict] = None
        self.bind("<Configure>", lambda e: self.redraw())

    def show(self, result: Optional[FileResult], plot: Optional[dict]) -> None:
        self.result, self.plot = result, plot
        self.redraw()

    def _message(self, text: str) -> None:
        self.create_text(self.winfo_width() / 2, self.winfo_height() / 2, text=text, fill="#888")

    def _freq_axis(self, nyq: float, x0: float, y_of) -> None:
        kstep = 2 if nyq <= 24000 else 5 if nyq <= 50000 else 10 if nyq <= 100000 else 20
        k = 0
        while k * 1000 <= nyq:
            y = y_of(k * 1000)
            self.create_line(x0 - 4, y, x0, y, fill="#666")
            self.create_text(x0 - 6, y, text=f"{k}k", anchor="e", fill="#555", font=("TkDefaultFont", 8))
            k += kstep


class SpectrogramCanvas(_PlotBase):
    """Time-frequency picture: an upsampled or lossy-sourced file shows a hard
    black area above its cut-off for the whole duration of the track."""

    def __init__(self, master, app: "App", **kw):
        super().__init__(master, **kw)
        self.app = app
        self.configure(background="#1e1e1e")
        self._img = None

    def redraw(self) -> None:
        from .imaging import spectrogram_rgb, to_ppm

        self.delete("all")
        w, h = self.winfo_width(), self.winfo_height()
        if w < 80 or h < 60:
            return
        sg = (self.plot or {}).get("spectrogram") if self.plot else None
        if self.result is None:
            self.create_text(w / 2, h / 2, text="Select a scanned file to see its spectrogram", fill="#aaa")
            return
        if not sg:
            self.create_text(w / 2, h / 2, text="No spectrogram (silent or unreadable file)", fill="#aaa")
            return
        L, R, T, B = self.L, self.R, self.T, self.B
        pw, ph = w - L - R, h - T - B
        cfg = self.app.cfg
        rgb = spectrogram_rgb(sg, pw, ph, cfg.spectrogram_range_db, cfg.colormap)
        self._img = tk.PhotoImage(data=to_ppm(rgb), format="PPM")
        self.create_image(L, T, image=self._img, anchor="nw")
        nyq = sg["nyquist"]

        def Y(f):
            return T + ph - ph * f / nyq

        self._freq_axis(nyq, L, Y)
        for c in self.find_withtag("all"):
            if self.type(c) == "text":
                self.itemconfigure(c, fill="#ccc")
        total = sg["shape"][0] * sg["seconds_per_column"]
        for frac in (0, 0.25, 0.5, 0.75, 1.0):
            x = L + frac * pw
            self.create_line(x, T + ph, x, T + ph + 4, fill="#aaa")
            self.create_text(x, T + ph + 13, text=fmt_duration(frac * total),
                             fill="#ccc", font=("TkDefaultFont", 8))
        r = self.result
        marks = []
        if r.upsampled_from:
            marks.append((r.upsampled_from / 2.0, "#4fc3f7",
                          f"{r.upsampled_from / 2000:g} kHz: limit of a {r.upsampled_from / 1000:g} kHz master"))
        if r.lossy_cutoff_hz:
            marks.append((r.lossy_cutoff_hz, "#ff6e6e", f"{r.lossy_cutoff_hz / 1000:.1f} kHz: lossy encoder cut-off"))
        for hz, color, label in marks:
            if 0 < hz < nyq:
                y = Y(hz)
                self.create_line(L, y, L + pw, y, fill=color, dash=(6, 4), width=2)
                self.create_text(L + pw - 4, y - 8, text=label, anchor="e", fill=color,
                                 font=("TkDefaultFont", 9, "bold"))
        self.create_text(L, 12, anchor="w", fill="#eee", font=("TkDefaultFont", 9, "bold"),
                         text="Spectrogram (time →, frequency ↑)")


class SpectrumCanvas(_PlotBase):
    """Average spectrum over the whole file."""

    def redraw(self) -> None:
        self.delete("all")
        w, h = self.winfo_width(), self.winfo_height()
        if w < 80 or h < 60:
            return
        p = self.plot
        if self.result is None or not p or not p.get("freqs"):
            self._message("Select a scanned file to see its spectrum" if self.result is None
                          else "No spectrum available")
            return
        L, R, T, B = self.L, self.R, self.T, self.B
        pw, ph = w - L - R, h - T - B
        nyq = p["nyquist"]
        top = (int(p["peak_db"] + 5) // 10 + 1) * 10
        bottom = max((int(min(p["floor_db"], p["threshold_db"]) - 15) // 10) * 10, top - 200)

        def X(f):
            return L + pw * f / nyq

        def Y(v):
            v = min(max(v, bottom), top)
            return T + ph * (top - v) / (top - bottom)

        step = 10 if top - bottom <= 120 else 20
        lvl = top
        while lvl >= bottom:
            self.create_line(L, Y(lvl), L + pw, Y(lvl), fill="#eeeeee")
            self.create_text(L - 6, Y(lvl), text=f"{lvl}", anchor="e", fill="#666", font=("TkDefaultFont", 8))
            lvl -= step
        kstep = 2 if nyq <= 24000 else 5 if nyq <= 50000 else 10 if nyq <= 100000 else 20
        k = 0
        while k * 1000 <= nyq:
            x = X(k * 1000)
            self.create_line(x, T, x, T + ph, fill="#eeeeee")
            self.create_text(x, T + ph + 12, text=f"{k}k", fill="#666", font=("TkDefaultFont", 8))
            k += kstep
        for i, ref in enumerate((22050, 24000, 44100, 48000)):
            if ref < nyq * 0.9:
                self.create_line(X(ref), T, X(ref), T + ph, fill="#aaaaaa", dash=(2, 3))
                self.create_text(X(ref) + 3, T + ph - 8 - 12 * (i % 2), text=f"{ref / 1000:g}k",
                                 anchor="w", fill="#999", font=("TkDefaultFont", 8))
        color = VERDICT_STYLE.get(self.result.verdict, ("", "#1565c0"))[1]
        if p.get("bandwidth_hz"):
            x = X(p["bandwidth_hz"])
            self.create_line(x, T, x, T + ph, fill=color, dash=(6, 4), width=2)
            anchor, dx = ("e", -4) if x > L + pw - 130 else ("w", 4)
            self.create_text(x + dx, T + 10, text=f"bandwidth {p['bandwidth_hz'] / 1000:.1f} kHz",
                             anchor=anchor, fill=color)
        self.create_line(L, Y(p["floor_db"]), L + pw, Y(p["floor_db"]), fill="#8250df", dash=(4, 4))
        pts = []
        for f, v in zip(p["freqs"], p["db"]):
            pts += [X(f), Y(v)]
        if len(pts) >= 4:
            self.create_line(*pts, fill="#1f2328", width=1)
        self.create_rectangle(L, T, L + pw, T + ph, outline="#999")
        self.create_text(L, 12, anchor="w", font=("TkDefaultFont", 9, "bold"), fill="#333",
                         text="Average spectrum (dB per bin)")


# ---------------------------------------------------------------- dialogs ---
class SummaryDialog(tk.Toplevel):
    """Pop-up shown after a scan: percentage of files per verdict."""

    def __init__(self, app: "App", results: List[FileResult], elapsed: Optional[float]):
        super().__init__(app.root)
        self.app = app
        self.title("Scan summary")
        self.transient(app.root)
        self.resizable(False, False)
        frm = ttk.Frame(self, padding=16)
        frm.grid(sticky="nsew")
        n = len(results)
        problems = sum(1 for r in results if r.verdict in PROBLEM_VERDICTS)
        head = f"Scanned {n} file{'s' if n != 1 else ''}"
        if elapsed:
            head += f" in {elapsed:.1f} s"
        ttk.Label(frm, text=head, font=("TkDefaultFont", 12, "bold")).grid(row=0, column=0, columnspan=4, sticky="w")
        pct_problems = 100.0 * problems / n if n else 0.0
        ttk.Label(frm, text=f"{problems} file{'s' if problems != 1 else ''} ({pct_problems:.1f} %) "
                            "are fake or suspicious." if problems else "No fake or suspicious files found.",
                  foreground="#a3122a" if problems else "#2e7d32").grid(row=1, column=0, columnspan=4,
                                                                         sticky="w", pady=(2, 10))
        bar_w = 260
        for i, (v, count, pct) in enumerate(verdict_breakdown(results)):
            label, color, desc = VERDICT_STYLE[v]
            ttk.Label(frm, text=label, foreground=color, font=("TkDefaultFont", 10, "bold")).grid(
                row=2 + 2 * i, column=0, sticky="w", padx=(0, 10))
            c = tk.Canvas(frm, width=bar_w, height=16, highlightthickness=0, background="#eeeeee")
            c.create_rectangle(0, 0, max(2, bar_w * pct / 100), 16, fill=color, outline="")
            c.grid(row=2 + 2 * i, column=1, sticky="w")
            ttk.Label(frm, text=f"{pct:5.1f} %", width=8, anchor="e").grid(row=2 + 2 * i, column=2)
            ttk.Label(frm, text=f"{count} file{'s' if count != 1 else ''}", foreground="#555").grid(
                row=2 + 2 * i, column=3, sticky="w", padx=(8, 0))
            ttk.Label(frm, text=desc, foreground="#777", font=("TkDefaultFont", 8)).grid(
                row=3 + 2 * i, column=0, columnspan=4, sticky="w", pady=(0, 6))
        self.again = tk.BooleanVar(value=app.cfg.show_summary_popup)
        ttk.Checkbutton(frm, text="Show this summary after every scan", variable=self.again).grid(
            row=100, column=0, columnspan=4, sticky="w", pady=(8, 8))
        btns = ttk.Frame(frm)
        btns.grid(row=101, column=0, columnspan=4, sticky="e")
        ttk.Button(btns, text="Show problem files", command=lambda: self._close(problems_only=True)).pack(side="left")
        ttk.Button(btns, text="Show all files", command=lambda: self._close(show_all=True)).pack(side="left", padx=4)
        ttk.Button(btns, text="Export CSV…", command=lambda: app.export("csv", all_results=True)).pack(side="left")
        close = ttk.Button(btns, text="Close", command=self._close)
        close.pack(side="left", padx=(4, 0))
        close.focus_set()
        self.bind("<Escape>", lambda e: self._close())
        self.protocol("WM_DELETE_WINDOW", self._close)
        self.update_idletasks()
        x = app.root.winfo_rootx() + (app.root.winfo_width() - self.winfo_width()) // 2
        y = app.root.winfo_rooty() + (app.root.winfo_height() - self.winfo_height()) // 3
        self.geometry(f"+{max(x, 0)}+{max(y, 0)}")

    def _close(self, problems_only: bool = False, show_all: bool = False) -> None:
        if self.again.get() != self.app.cfg.show_summary_popup:
            self.app.cfg.show_summary_popup = self.again.get()
            self.app.save_settings()
        if problems_only:
            self.app.set_filters(PROBLEM_VERDICTS)
        elif show_all:
            self.app.set_filters(VERDICT_ORDER)
        self.destroy()


class SettingsDialog(tk.Toplevel):
    """Edit every field of settings.Settings, grouped in tabs."""

    def __init__(self, app: "App"):
        super().__init__(app.root)
        self.app = app
        self.title("Settings")
        self.transient(app.root)
        self.vars: Dict[str, object] = {}
        outer = ttk.Frame(self, padding=12)
        outer.grid(sticky="nsew")
        self.columnconfigure(0, weight=1)
        self.rowconfigure(0, weight=1)
        outer.columnconfigure(0, weight=1)
        outer.rowconfigure(0, weight=1)
        nb = ttk.Notebook(outer)
        nb.grid(row=0, column=0, sticky="nsew")
        self.help_var = tk.StringVar(value="Point at a setting to see what it does.")
        infos = settings_mod.Settings.field_info()
        for group in settings_mod.GROUPS:
            tab = ttk.Frame(nb, padding=10)
            tab.columnconfigure(1, weight=1)
            nb.add(tab, text=group)
            row = 0
            if group == "Detection":
                ttk.Label(tab, text="Changes apply to the next scan.", foreground="#666").grid(
                    row=row, column=0, columnspan=2, sticky="w", pady=(0, 6))
                row += 1
            for info in [i for i in infos if i["group"] == group]:
                widget = self._widget(tab, info)
                lab = ttk.Label(tab, text=info["label"])
                lab.grid(row=row, column=0, sticky="nw", padx=(0, 12), pady=3)
                widget.grid(row=row, column=1, sticky="w", pady=3)
                for wdg in (lab, widget):
                    wdg.bind("<Enter>", lambda e, h=info["help"]: self.help_var.set(h))
                    wdg.bind("<FocusIn>", lambda e, h=info["help"]: self.help_var.set(h), add="+")
                row += 1
        ttk.Label(outer, textvariable=self.help_var, wraplength=520, foreground="#444",
                  justify="left").grid(row=1, column=0, sticky="ew", pady=(8, 4))
        ttk.Label(outer, text=f"Saved in {settings_mod.default_path()}", foreground="#888",
                  font=("TkDefaultFont", 8)).grid(row=2, column=0, sticky="w")
        btns = ttk.Frame(outer)
        btns.grid(row=3, column=0, sticky="ew", pady=(10, 0))
        ttk.Button(btns, text="Restore defaults", command=self._defaults).pack(side="left")
        ttk.Button(btns, text="Save", command=self._save).pack(side="right")
        ttk.Button(btns, text="Cancel", command=self.destroy).pack(side="right", padx=4)
        self.bind("<Escape>", lambda e: self.destroy())
        self._load(app.cfg)
        self.update_idletasks()
        self.minsize(self.winfo_width(), self.winfo_height())
        self.grab_set()

    def _widget(self, parent, info):
        kind, name = info.get("kind"), info["name"]
        if kind == "bool":
            v = tk.BooleanVar()
            self.vars[name] = v
            return ttk.Checkbutton(parent, variable=v)
        if kind == "choice":
            v = tk.StringVar()
            self.vars[name] = v
            return ttk.Combobox(parent, textvariable=v, values=info["choices"], state="readonly", width=18)
        if kind == "multi":
            frame = ttk.Frame(parent)
            sub = {}
            for i, choice in enumerate(info["choices"]):
                b = tk.BooleanVar()
                sub[choice] = b
                label = VERDICT_STYLE.get(choice, (choice,))[0]
                ttk.Checkbutton(frame, text=label, variable=b).grid(row=i // 4, column=i % 4, sticky="w", padx=(0, 8))
            self.vars[name] = sub
            return frame
        v = tk.StringVar()
        self.vars[name] = v
        if kind == "str":
            return ttk.Entry(parent, textvariable=v, width=60)
        inc = 1 if kind == "int" or (info.get("max") or 0) > 50 else 0.1
        return ttk.Spinbox(parent, textvariable=v, from_=info.get("min") or 0, to=info.get("max") or 1e9,
                           increment=inc, width=10)

    def _load(self, cfg) -> None:
        for name, var in self.vars.items():
            value = getattr(cfg, name)
            if isinstance(var, dict):
                for choice, b in var.items():
                    b.set(choice in value)
            elif isinstance(var, tk.BooleanVar):
                var.set(bool(value))
            else:
                var.set(f"{value:g}" if isinstance(value, float) else str(value))

    def _defaults(self) -> None:
        self._load(settings_mod.Settings())

    def _save(self) -> None:
        data, bad = {}, []
        infos = {i["name"]: i for i in settings_mod.Settings.field_info()}
        for name, var in self.vars.items():
            if isinstance(var, dict):
                data[name] = [c for c, b in var.items() if b.get()]
            else:
                data[name] = var.get()
        new = settings_mod.Settings.from_dict(data)
        for name, raw in data.items():
            if isinstance(raw, str) and infos[name].get("kind") not in ("str", "choice"):
                try:
                    ok = float(raw) == float(getattr(new, name))
                except ValueError:
                    ok = False
                if not ok:
                    lo, hi = infos[name].get("min"), infos[name].get("max")
                    bad.append(f"{infos[name]['label']}: '{raw}' (allowed {lo:g} – {hi:g})")
        if bad:
            messagebox.showerror("Invalid settings", "Please fix:\n\n" + "\n".join(bad), parent=self)
            return
        self.app.apply_settings(new)
        self.destroy()


# -------------------------------------------------------------------- app ---
class App(ttk.Frame):
    COLUMNS = ("verdict", "why", "claimed", "estimated", "bandwidth", "name", "folder")
    HEADINGS = {"verdict": "Verdict", "why": "Why", "claimed": "Claimed", "estimated": "Real (est.)",
                "bandwidth": "Bandwidth", "name": "File", "folder": "Folder"}

    def __init__(self, root: tk.Tk, initial: Optional[List[str]] = None, cfg=None):
        super().__init__(root, padding=8)
        self.root = root
        self.cfg = cfg or settings_mod.load()
        self.sources: List[str] = []                 # added folders/files, in order
        self.source_files: Dict[str, List[str]] = {}  # source -> listed audio files (absolute)
        self.listing: set = set()                     # sources still being listed
        self.scan_when_listed = False
        self.results: Dict[str, FileResult] = {}
        self.cache = PlotCache()
        self.queue: queue.Queue = queue.Queue()
        self.scanner: Optional[Scanner] = None
        self.total = self.done = 0
        self.scanning = False
        self.scan_started = 0.0
        self.last_elapsed: Optional[float] = None
        self.sort_col, self.sort_rev = "verdict", False
        self.show_vars = {v: tk.BooleanVar(value=v in self.cfg.shown_verdicts) for v in VERDICT_ORDER}
        self.search_var = tk.StringVar()
        self.quick_var = tk.BooleanVar(value=False)
        self.status_var = tk.StringVar(value="Add a folder or files, then press Scan.")
        self._build_menu()
        self._build()
        self.grid(sticky="nsew")
        root.columnconfigure(0, weight=1)
        root.rowconfigure(0, weight=1)
        root.protocol("WM_DELETE_WINDOW", self.quit_app)
        if initial:
            for p in initial:
                self._add_source(p)
            self.after(200, self.start_scan)  # waits for the listing to finish
        self.after(100, self._poll)

    # ---------- layout ---------------------------------------------------
    def _build_menu(self) -> None:
        mod = "Command" if sys.platform == "darwin" else "Control"
        acc = "Cmd" if sys.platform == "darwin" else "Ctrl"
        m = tk.Menu(self.root)
        f = tk.Menu(m, tearoff=False)
        f.add_command(label="Add Folder…", accelerator=f"{acc}+O", command=self.add_folder)
        f.add_command(label="Add Files…", command=self.add_files)
        f.add_separator()
        f.add_command(label="Export listed files as CSV…", command=lambda: self.export("csv"))
        f.add_command(label="Export listed files as JSON…", command=lambda: self.export("json"))
        f.add_separator()
        f.add_command(label="Quit", accelerator=f"{acc}+Q", command=self.quit_app)
        m.add_cascade(label="File", menu=f)
        e = tk.Menu(m, tearoff=False)
        e.add_command(label="Open file location", accelerator="Enter", command=self.reveal)
        e.add_command(label="Open file", accelerator=f"{acc}+Enter", command=self.open_file)
        e.add_command(label="Copy path", accelerator=f"{acc}+C", command=self.copy_path)
        e.add_separator()
        e.add_command(label="Settings…", accelerator=f"{acc}+,", command=self.open_settings)
        m.add_cascade(label="Edit", menu=e)
        v = tk.Menu(m, tearoff=False)
        v.add_command(label="Scan summary…", command=self.show_summary)
        v.add_command(label="Show problem files only", command=lambda: self.set_filters(PROBLEM_VERDICTS))
        v.add_command(label="Show all files", command=lambda: self.set_filters(VERDICT_ORDER))
        m.add_cascade(label="View", menu=v)
        h = tk.Menu(m, tearoff=False)
        h.add_command(label="About Real-Hi-Res", command=self.about)
        m.add_cascade(label="Help", menu=h)
        self.root.configure(menu=m)
        self.root.bind_all(f"<{mod}-o>", lambda e: self.add_folder())
        self.root.bind_all(f"<{mod}-q>", lambda e: self.quit_app())
        self.root.bind_all(f"<{mod}-comma>", lambda e: self.open_settings())

    def _build(self) -> None:
        self.columnconfigure(0, weight=1)
        bar = ttk.Frame(self)
        bar.grid(row=0, column=0, sticky="ew")
        ttk.Button(bar, text="Add Folder…", command=self.add_folder).pack(side="left")
        ttk.Button(bar, text="Add Files…", command=self.add_files).pack(side="left", padx=(4, 0))
        ttk.Button(bar, text="Clear", command=self.clear).pack(side="left", padx=(4, 12))
        self.quick_cb = ttk.Checkbutton(bar, variable=self.quick_var)
        self.quick_cb.pack(side="left")
        self._update_quick_label()
        ttk.Button(bar, text="⚙ Settings…", command=self.open_settings).pack(side="right")
        self.stop_btn = ttk.Button(bar, text="Stop", command=self.stop_scan, state="disabled")
        self.stop_btn.pack(side="right", padx=(0, 12))
        self.scan_btn = ttk.Button(bar, text="Scan", command=self.start_scan)
        self.scan_btn.pack(side="right", padx=(0, 4))

        src = ttk.LabelFrame(self, text="Sources to scan", padding=(6, 4))
        src.grid(row=1, column=0, sticky="ew", pady=(6, 4))
        src.columnconfigure(0, weight=1)
        self.src_tree = ttk.Treeview(src, columns=("kind", "files", "problems"), height=3,
                                     show="tree headings", selectmode="extended")
        self.src_tree.heading("#0", text="Folder / file", anchor="w")
        self.src_tree.heading("kind", text="Type")
        self.src_tree.heading("files", text="Audio files")
        self.src_tree.heading("problems", text="Fake / suspicious")
        self.src_tree.column("#0", width=560, stretch=True)
        self.src_tree.column("kind", width=70, anchor="center", stretch=False)
        self.src_tree.column("files", width=150, anchor="center", stretch=False)
        self.src_tree.column("problems", width=130, anchor="center", stretch=False)
        self.src_tree.tag_configure("empty", foreground="#c62828")
        self.src_tree.tag_configure("busy", foreground="#808080")
        sys_ = ttk.Scrollbar(src, orient="vertical", command=self.src_tree.yview)
        self.src_tree.configure(yscrollcommand=sys_.set)
        self.src_tree.grid(row=0, column=0, sticky="ew")
        sys_.grid(row=0, column=1, sticky="ns")
        side = ttk.Frame(src)
        side.grid(row=0, column=2, sticky="n", padx=(6, 0))
        self.remove_btn = ttk.Button(side, text="Remove", command=self.remove_sources)
        self.remove_btn.pack(fill="x")
        self.src_hint = ttk.Label(src, foreground="#666",
                                  text="Nothing added yet. Click “Add Folder…” or “Add Files…”: the files "
                                       "found are listed below straight away, then press Scan.")
        self.src_hint.grid(row=1, column=0, columnspan=3, sticky="w", pady=(4, 0))
        self.src_tree.bind("<Delete>", lambda e: self.remove_sources())
        self.src_tree.bind("<BackSpace>", lambda e: self.remove_sources())

        filt = ttk.Frame(self)
        filt.grid(row=2, column=0, sticky="ew", pady=(0, 4))
        ttk.Label(filt, text="Show:").pack(side="left")
        self.filter_checks: Dict[str, ttk.Checkbutton] = {}
        for v in VERDICT_ORDER:
            cb = ttk.Checkbutton(filt, text=VERDICT_STYLE[v][0], variable=self.show_vars[v],
                                 command=self.refresh)
            cb.pack(side="left", padx=(6, 0))
            self.filter_checks[v] = cb
        entry = ttk.Entry(filt, textvariable=self.search_var, width=24)
        entry.pack(side="right")
        ttk.Label(filt, text="Search:").pack(side="right", padx=(0, 4))
        self.search_var.trace_add("write", lambda *a: self.refresh())

        panes = ttk.PanedWindow(self, orient="vertical")
        panes.grid(row=3, column=0, sticky="nsew")
        self.rowconfigure(3, weight=1)

        # result list
        table = ttk.Frame(panes)
        table.columnconfigure(0, weight=1)
        table.rowconfigure(0, weight=1)
        self.tree = ttk.Treeview(table, columns=self.COLUMNS, show="headings", selectmode="extended",
                                 height=7)
        widths = {"verdict": 105, "why": 400, "claimed": 70, "estimated": 85, "bandwidth": 82,
                  "name": 230, "folder": 220}
        for c in self.COLUMNS:
            self.tree.heading(c, text=self.HEADINGS[c], command=lambda c=c: self.sort_by(c))
            self.tree.column(c, width=widths[c], stretch=c in ("why", "name", "folder"),
                             anchor="w" if c in ("name", "folder", "verdict", "why") else "center")
        for v, (_, color, _) in VERDICT_STYLE.items():
            self.tree.tag_configure(v, foreground=color)
        self.tree.tag_configure(PENDING, foreground=PENDING_STYLE[1])
        ys = ttk.Scrollbar(table, orient="vertical", command=self.tree.yview)
        xs = ttk.Scrollbar(table, orient="horizontal", command=self.tree.xview)
        self.tree.configure(yscrollcommand=ys.set, xscrollcommand=xs.set)
        self.tree.grid(row=0, column=0, sticky="nsew")
        ys.grid(row=0, column=1, sticky="ns")
        xs.grid(row=1, column=0, sticky="ew")
        self.tree.bind("<<TreeviewSelect>>", lambda e: self.show_details())
        self.tree.bind("<Double-1>", lambda e: self._primary_action())
        self.tree.bind("<Return>", lambda e: self._primary_action())
        mod = "Command" if sys.platform == "darwin" else "Control"
        self.tree.bind(f"<{mod}-Return>", lambda e: self._secondary_action())
        self.tree.bind(f"<{mod}-c>", lambda e: self.copy_path())
        self.tree.bind("<Button-3>", self._context_menu)
        self.tree.bind("<Button-2>", self._context_menu)  # macOS right click
        panes.add(table, weight=3)

        # dashboard
        dash = ttk.PanedWindow(panes, orient="horizontal")
        info = ttk.Frame(dash, padding=(4, 6, 8, 4))
        info.columnconfigure(1, weight=1)
        info.rowconfigure(5, weight=1)
        self.badge = tk.Label(info, text="", fg="white", bg="#9e9e9e", padx=10, pady=3,
                              font=("TkDefaultFont", 11, "bold"))
        self.badge.grid(row=0, column=0, sticky="w")
        self.name_var = tk.StringVar(value="Select a file in the list")
        ttk.Label(info, textvariable=self.name_var, font=("TkDefaultFont", 10, "bold")).grid(
            row=0, column=1, sticky="w", padx=(8, 0))
        self.why_var = tk.StringVar(value="")
        self.why_label = ttk.Label(info, textvariable=self.why_var, wraplength=420, justify="left",
                                   font=("TkDefaultFont", 10))
        self.why_label.grid(row=1, column=0, columnspan=2, sticky="ew", pady=(8, 6))
        facts = ttk.Frame(info)
        facts.grid(row=2, column=0, columnspan=2, sticky="ew")
        self.fact_vars: Dict[str, tk.StringVar] = {}
        for i, key in enumerate(("Claimed", "Real (estimated)", "Bandwidth", "Bits used", "Format",
                                 "Duration")):
            ttk.Label(facts, text=key + ":", foreground="#666").grid(row=i // 2, column=(i % 2) * 2,
                                                                       sticky="w", padx=(0, 4))
            var = tk.StringVar(value="–")
            self.fact_vars[key] = var
            ttk.Label(facts, textvariable=var).grid(row=i // 2, column=(i % 2) * 2 + 1, sticky="w",
                                                    padx=(0, 18))
        btns = ttk.Frame(info)
        btns.grid(row=3, column=0, columnspan=2, sticky="w", pady=(8, 6))
        self.open_folder_btn = ttk.Button(btns, text="Open Folder", command=self.reveal, state="disabled")
        self.open_folder_btn.pack(side="left")
        self.open_file_btn = ttk.Button(btns, text="Open File", command=self.open_file, state="disabled")
        self.open_file_btn.pack(side="left", padx=4)
        self.copy_btn = ttk.Button(btns, text="Copy Path", command=self.copy_path, state="disabled")
        self.copy_btn.pack(side="left")
        ttk.Label(info, text="Details", foreground="#666").grid(row=4, column=0, columnspan=2, sticky="w")
        txt_frame = ttk.Frame(info)
        txt_frame.grid(row=5, column=0, columnspan=2, sticky="nsew")
        txt_frame.rowconfigure(0, weight=1)
        txt_frame.columnconfigure(0, weight=1)
        self.text = tk.Text(txt_frame, wrap="word", height=4, relief="flat", padx=6, pady=4,
                            background="#fafafa", foreground="#1f2328", font=("TkDefaultFont", 9))
        tys = ttk.Scrollbar(txt_frame, orient="vertical", command=self.text.yview)
        self.text.configure(yscrollcommand=tys.set, state="disabled")
        self.text.grid(row=0, column=0, sticky="nsew")
        tys.grid(row=0, column=1, sticky="ns")
        dash.add(info, weight=2)
        info.bind("<Configure>", lambda e: self.why_label.configure(wraplength=max(200, e.width - 20)))

        plots = ttk.Notebook(dash)
        self.spectrogram = SpectrogramCanvas(plots, self, height=200)
        self.spectrum = SpectrumCanvas(plots, height=200)
        plots.add(self.spectrogram, text="Spectrogram")
        plots.add(self.spectrum, text="Average spectrum")
        dash.add(plots, weight=3)
        panes.add(dash, weight=2)

        status = ttk.Frame(self)
        status.grid(row=4, column=0, sticky="ews", pady=(6, 0))
        self.rowconfigure(4, weight=0, minsize=24)
        status.columnconfigure(1, weight=1)
        self.progress = ttk.Progressbar(status, length=220, mode="determinate")
        self.progress.grid(row=0, column=0, padx=(0, 8))
        ttk.Label(status, textvariable=self.status_var).grid(row=0, column=1, sticky="w")

        self.menu = tk.Menu(self.root, tearoff=False)
        self.menu.add_command(label="Open file location", command=self.reveal)
        self.menu.add_command(label="Open file", command=self.open_file)
        self.menu.add_command(label="Copy path", command=self.copy_path)

    def _update_quick_label(self) -> None:
        self.quick_cb.configure(text=f"Quick scan (first {self.cfg.quick_scan_seconds:g} s)")

    # ---------- settings -------------------------------------------------
    def open_settings(self) -> None:
        SettingsDialog(self)

    def save_settings(self) -> None:
        try:
            settings_mod.save(self.cfg)
        except OSError as exc:
            messagebox.showwarning("Settings", f"Could not save settings: {exc}")

    def apply_settings(self, new) -> None:
        old_shown = set(self.cfg.shown_verdicts)
        relist = (new.extension_set() != self.cfg.extension_set() or new.recursive != self.cfg.recursive)
        self.cfg = new
        if relist and not self.scanning:  # file list depends on these: list the sources again
            sources = list(self.sources)
            self.clear()
            for src in sources:
                self._add_source(src)
        self.save_settings()
        if set(new.shown_verdicts) != old_shown:
            self.set_filters(new.shown_verdicts)
        self._update_quick_label()
        self.spectrogram.redraw()
        self.status_var.set("Settings saved. Detection changes apply to the next scan.")

    # ---------- sources --------------------------------------------------
    def _add_source(self, path: str) -> None:
        """Add a folder/file and list its audio files right away (in the
        background), so the user sees exactly what will be scanned."""
        if not path:
            return
        path = os.path.abspath(path)
        if path in self.sources:
            self.status_var.set(f"Already added: {path}")
            if self.src_tree.exists(path):
                self.src_tree.selection_set(path)
                self.src_tree.see(path)
            return
        self.sources.append(path)
        kind = "Folder" if os.path.isdir(path) else "File"
        self.src_tree.insert("", "end", iid=path, text=path, values=(kind, "counting…", ""),
                             tags=("busy",))
        self.src_tree.see(path)
        self.src_hint.grid_remove()
        self.src_tree.configure(height=min(max(len(self.sources), 2), 5))
        self.listing.add(path)
        self.status_var.set(f"Looking for audio files in {path} …")
        threading.Thread(target=list_source, args=(path, self.cfg, self.queue), daemon=True).start()

    def _on_listed(self, source: str, files: List[str]) -> None:
        self.listing.discard(source)
        if source not in self.sources:  # removed while listing
            return
        self.source_files[source] = files
        if self.src_tree.exists(source):
            n = len(files)
            if n:
                self.src_tree.item(source, values=(self.src_tree.set(source, "kind"),
                                                   f"{n} file{'s' if n != 1 else ''}", ""), tags=())
            else:
                why = ("not an audio file (check Settings → Scanning → File extensions)"
                       if os.path.isfile(source) else "no audio files found")
                self.src_tree.item(source, values=(self.src_tree.set(source, "kind"), why, ""),
                                   tags=("empty",))
        self.refresh()
        if not self.listing:
            total = len(self.all_files())
            empty = [s for s in self.sources if not self.source_files.get(s)]
            msg = (f"{total} audio file{'s' if total != 1 else ''} ready in {len(self.sources)} "
                   f"source{'s' if len(self.sources) != 1 else ''}. Press Scan.")
            if empty:
                msg += f"  ({len(empty)} source(s) contain no audio files.)"
            if not self.scanning:
                self.status_var.set(msg)
            if self.scan_when_listed:
                self.scan_when_listed = False
                self.start_scan()

    def all_files(self) -> List[str]:
        seen, out = set(), []
        for src in self.sources:
            for f in self.source_files.get(src, []):
                if f not in seen:
                    seen.add(f)
                    out.append(f)
        return out

    def remove_sources(self) -> None:
        if self.scanning:
            self.status_var.set("Stop the scan before removing sources.")
            return
        sel = list(self.src_tree.selection())
        if not sel:
            self.status_var.set("Select a source in the “Sources to scan” list to remove it.")
            return
        for src in sel:
            if src in self.sources:
                self.sources.remove(src)
            self.source_files.pop(src, None)
            self.listing.discard(src)
            self.src_tree.delete(src)
        keep = set(self.all_files())
        for path in [p for p in self.results if p not in keep]:
            del self.results[path]
        if not self.sources:
            self.src_hint.grid()
        self.refresh()
        self.show_details()
        n = len(keep)
        self.status_var.set(f"Removed {len(sel)} source(s). {n} audio file{'s' if n != 1 else ''} left to scan.")

    def _update_source_counts(self) -> None:
        for src in self.sources:
            if not self.src_tree.exists(src) or src in self.listing:
                continue
            files = self.source_files.get(src, [])
            done = [self.results[f] for f in files if f in self.results]
            probs = sum(1 for r in done if r.is_problem)
            self.src_tree.set(src, "problems", f"{probs} of {len(done)}" if done else "")

    def add_folder(self) -> None:
        d = filedialog.askdirectory(title="Choose a music folder")
        if d:
            self._add_source(d)

    def add_files(self) -> None:
        pattern = " ".join(f"*{e}" for e in sorted(self.cfg.extension_set()))
        files = filedialog.askopenfilenames(title="Choose audio files",
                                            filetypes=[("Audio files", pattern), ("All files", "*")])
        for f in files:
            self._add_source(f)

    def clear(self) -> None:
        if self.scanning:
            return
        self.sources.clear()
        self.source_files.clear()
        self.listing.clear()
        self.results.clear()
        self.cache.clear()
        self.src_tree.delete(*self.src_tree.get_children())
        self.src_hint.grid()
        self.refresh()
        self.show_details()
        self.progress["value"] = 0
        for v, cb in self.filter_checks.items():
            cb.configure(text=VERDICT_STYLE[v][0])
        self.status_var.set("Add a folder or files, then press Scan.")

    # ---------- scanning -------------------------------------------------
    def start_scan(self) -> None:
        if self.scanning:
            return
        if not self.sources:
            messagebox.showinfo("Real-Hi-Res", "Add a folder or some files first.")
            return
        if self.listing:
            self.scan_when_listed = True
            self.status_var.set("Still looking for audio files… the scan starts as soon as the list is complete.")
            return
        files = self.all_files()
        if not files:
            messagebox.showinfo("Real-Hi-Res", "The added folders/files contain no audio files.\n\n"
                                "Check the file extensions under Settings → Scanning.")
            return
        try:
            require_tools()
        except ToolMissingError as exc:
            messagebox.showerror("FFmpeg not found", str(exc))
            return
        self.results.clear()
        self.cache.clear()
        self.total, self.done = len(files), 0
        self.scanning = True
        self.refresh()
        self.show_details()
        self._update_source_counts()
        self.scan_started = time.time()
        self.progress.configure(mode="determinate", maximum=max(len(files), 1), value=0)
        self.status_var.set(f"Scanning {len(files)} files…")
        self.scan_btn.configure(state="disabled")
        self.stop_btn.configure(state="normal")
        max_seconds = self.cfg.quick_scan_seconds if self.quick_var.get() else (self.cfg.max_seconds or None)
        self.scanner = Scanner(files, self.cfg, max_seconds, self.queue)
        self.scanner.start()

    def stop_scan(self) -> None:
        if self.scanner:
            self.scanner.stop_event.set()
            self.status_var.set("Stopping after the files already in progress…")
            self.stop_btn.configure(state="disabled")

    def _poll(self) -> None:
        dirty = False
        try:
            for _ in range(500):
                kind, payload, *rest = self.queue.get_nowait()
                if kind == "total":
                    self.total = payload
                    self.progress.stop()
                    self.progress.configure(mode="determinate", maximum=max(payload, 1), value=0)
                    if payload == 0:
                        self.status_var.set("No audio files found in the selected sources.")
                elif kind == "listed":
                    self._on_listed(payload, rest[0])
                elif kind == "result":
                    self.done += 1
                    self.cache.put(payload.path, payload.plot)
                    payload.plot = None
                    self.results[payload.path] = payload
                    self._insert(payload)
                    dirty = True
                elif kind == "done":
                    self._finish(stopped=payload == "stopped")
                elif kind == "error":
                    self._finish(stopped=True)
                    messagebox.showerror("Scan failed", payload)
        except queue.Empty:
            pass
        if dirty and self.scanning:
            self.progress["value"] = self.done
            self._update_status()
            self._update_source_counts()
        self.after(100, self._poll)

    def _finish(self, stopped: bool) -> None:
        self.scanning = False
        self.last_elapsed = time.time() - self.scan_started
        self.progress.stop()
        self.progress.configure(mode="determinate", maximum=max(self.total, 1), value=self.done)
        self.scan_btn.configure(state="normal")
        self.stop_btn.configure(state="disabled")
        self._update_status(final=True, stopped=stopped)
        self.refresh()
        self._update_source_counts()
        kids = self.tree.get_children()
        if kids and not self.tree.selection():
            self.tree.selection_set(kids[0])
            self.tree.focus(kids[0])
        if self.results and self.cfg.show_summary_popup:
            self.after(150, self.show_summary)

    def show_summary(self) -> None:
        if not self.results:
            messagebox.showinfo("Scan summary", "Nothing scanned yet.")
            return
        SummaryDialog(self, list(self.results.values()), self.last_elapsed)

    def _counts(self) -> Dict[str, int]:
        c = {v: 0 for v in VERDICT_ORDER}
        for r in self.results.values():
            c[r.verdict] = c.get(r.verdict, 0) + 1
        return c

    def _update_status(self, final: bool = False, stopped: bool = False) -> None:
        c = self._counts()
        for v, cb in self.filter_checks.items():
            cb.configure(text=f"{VERDICT_STYLE[v][0]} ({c[v]})")
        head = ("Stopped" if stopped else "Finished") if final else "Scanning"
        n = max(self.done, 1)
        fakes = c[FAKE] + c[FAKE_LOSSLESS]
        self.status_var.set(f"{head}: {self.done}/{self.total} files  ·  {fakes} fake "
                            f"({100 * fakes / n:.0f} %), {c[SUSPECT]} suspicious, {c[REAL]} real hi-res")

    # ---------- table ----------------------------------------------------
    def set_filters(self, verdicts) -> None:
        for v, var in self.show_vars.items():
            var.set(v in verdicts)
        self.refresh()

    def _matches(self, r: FileResult) -> bool:
        if not self.show_vars.get(r.verdict, tk.BooleanVar(value=True)).get():
            return False
        q = self.search_var.get().strip().lower()
        return not q or q in r.path.lower() or q in r.summary.lower()

    def _values(self, r: FileResult):
        bw = f"{r.bandwidth_hz / 1000:.1f} kHz" if r.bandwidth_hz else "-"
        return (VERDICT_STYLE[r.verdict][0], r.summary, r.claimed, r.estimated, bw,
                os.path.basename(r.path), os.path.dirname(r.path))

    def _pending_values(self, path: str):
        why = "Waiting in the scan queue…" if self.scanning else "Not scanned yet: press Scan"
        return (PENDING_STYLE[0], why, "", "", "", os.path.basename(path), os.path.dirname(path))

    def _search_ok(self, path: str, summary: str = "") -> bool:
        q = self.search_var.get().strip().lower()
        return not q or q in path.lower() or q in summary.lower()

    def _insert(self, r: FileResult) -> None:
        """Show a new result: replace its "not scanned" row, or hide it when
        the current filters exclude its verdict."""
        exists = self.tree.exists(r.path)
        if self._matches(r):
            if exists:
                self.tree.item(r.path, values=self._values(r), tags=(r.verdict,))
            else:
                self.tree.insert("", "end", iid=r.path, values=self._values(r), tags=(r.verdict,))
        elif exists:
            was_selected = r.path in self.tree.selection()
            self.tree.delete(r.path)
            if was_selected:
                self.show_details()

    def _sort_key(self, r: FileResult):
        c = self.sort_col
        if c == "verdict":
            return (VERDICT_ORDER.index(r.verdict), r.path.lower())
        if c == "bandwidth":
            return (r.bandwidth_hz or 0.0, r.path.lower())
        if c == "claimed":
            return (r.sample_rate, r.claimed_bits or 0, r.path.lower())
        if c == "estimated":
            return (r.true_rate or 0, r.effective_bits or 0, r.path.lower())
        if c == "name":
            return os.path.basename(r.path).lower()
        if c == "why":
            return (r.summary.lower(), r.path.lower())
        return r.path.lower()

    def sort_by(self, col: str) -> None:
        self.sort_rev = (not self.sort_rev) if col == self.sort_col else False
        self.sort_col = col
        self.refresh()

    def refresh(self) -> None:
        selected = set(self.tree.selection())
        self.tree.delete(*self.tree.get_children())
        rows = sorted((r for r in self.results.values() if self._matches(r)),
                      key=self._sort_key, reverse=self.sort_rev)
        for r in rows:
            self.tree.insert("", "end", iid=r.path, values=self._values(r), tags=(r.verdict,))
        # Listed files that have not been scanned yet are always shown, so the
        # user can see what was added.
        for path in self.all_files():
            if path not in self.results and self._search_ok(path) and not self.tree.exists(path):
                self.tree.insert("", "end", iid=path, values=self._pending_values(path), tags=(PENDING,))
        keep = [i for i in selected if self.tree.exists(i)]
        if keep:
            self.tree.selection_set(keep)
        for c in self.COLUMNS:
            arrow = (" ▼" if self.sort_rev else " ▲") if c == self.sort_col else ""
            self.tree.heading(c, text=self.HEADINGS[c] + arrow)
        if self.results:
            self._update_status(final=not self.scanning)

    # ---------- dashboard ------------------------------------------------
    def _current(self) -> Optional[FileResult]:
        sel = self.tree.selection()
        return self.results.get(sel[0]) if sel else None

    def _current_path(self) -> Optional[str]:
        sel = self.tree.selection()
        return sel[0] if sel else None

    def _set_text(self, parts) -> None:
        self.text.configure(state="normal")
        self.text.delete("1.0", "end")
        for chunk in parts:
            self.text.insert("end", chunk)
        self.text.configure(state="disabled")

    def show_details(self) -> None:
        r = self._current()
        path = self._current_path()
        state = "normal" if path else "disabled"
        for b in (self.open_folder_btn, self.open_file_btn, self.copy_btn):
            b.configure(state=state)
        if r is None and path:  # listed but not scanned yet
            self.badge.configure(text=PENDING_STYLE[0].upper(), bg=PENDING_STYLE[1])
            self.name_var.set(os.path.basename(path))
            self.why_var.set("Waiting in the scan queue…" if self.scanning else
                             "This file will be analysed when you press Scan.")
            for v in self.fact_vars.values():
                v.set("–")
            self._set_text([path + "\n"])
            self.spectrogram.show(None, None)
            self.spectrum.show(None, None)
            return
        if r is None:
            self.badge.configure(text="", bg="#9e9e9e")
            self.name_var.set("Select a file in the list")
            self.why_var.set("")
            for v in self.fact_vars.values():
                v.set("–")
            self._set_text([])
            self.spectrogram.show(None, None)
            self.spectrum.show(None, None)
            return
        label, color, _ = VERDICT_STYLE[r.verdict]
        self.badge.configure(text=label.upper(), bg=color)
        self.name_var.set(os.path.basename(r.path))
        self.why_var.set(r.summary or (r.reasons[0] if r.reasons else ""))
        bits = r.effective_bits
        if bits is not None and r.is_float and bits > 24:
            bits = 24
        self.fact_vars["Claimed"].set(r.claimed)
        self.fact_vars["Real (estimated)"].set(r.estimated)
        self.fact_vars["Bandwidth"].set(f"{r.bandwidth_hz / 1000:.1f} kHz" if r.bandwidth_hz else "–")
        self.fact_vars["Bits used"].set(f"{bits} of {r.claimed_bits}" if bits and r.claimed_bits else
                                        (str(bits) if bits else "–"))
        self.fact_vars["Format"].set(f"{r.codec}, {r.channels} ch" if r.codec else "–")
        self.fact_vars["Duration"].set(fmt_duration(r.duration) if r.duration else "?")
        parts = [f"• {x}\n" for x in r.reasons] + [f"• {x}\n" for x in r.notes]
        if r.cliffs_db:
            parts.append("• Drop at source Nyquists: " + ", ".join(
                f"{k / 2000:g} kHz {v:.0f} dB" for k, v in r.cliffs_db.items() if v == v) + "\n")
        parts.append(f"\n{r.path}\n")
        self._set_text(parts)
        plot = self.cache.get(r.path)
        self.spectrogram.show(r, plot)
        self.spectrum.show(r, plot)

    def _primary_action(self) -> None:
        (self.open_file if self.cfg.double_click == "Open file" else self.reveal)()

    def _secondary_action(self) -> None:
        (self.reveal if self.cfg.double_click == "Open file" else self.open_file)()

    def reveal(self) -> None:
        path = self._current_path()
        if path:
            try:
                reveal_in_file_manager(path)
            except OSError as exc:
                messagebox.showerror("Real-Hi-Res", f"Could not open the folder: {exc}")

    def open_file(self) -> None:
        path = self._current_path()
        if path:
            try:
                open_with_default_app(path)
            except OSError as exc:
                messagebox.showerror("Real-Hi-Res", f"Could not open the file: {exc}")

    def copy_path(self) -> None:
        paths = list(self.tree.selection())
        if paths:
            self.root.clipboard_clear()
            self.root.clipboard_append("\n".join(paths))
            self.status_var.set(f"Copied {len(paths)} path(s) to the clipboard.")

    def _context_menu(self, event) -> None:
        row = self.tree.identify_row(event.y)
        if row:
            if row not in self.tree.selection():
                self.tree.selection_set(row)
            self.menu.tk_popup(event.x_root, event.y_root)

    def export(self, kind: str, all_results: bool = False) -> None:
        if not self.results:
            messagebox.showinfo("Real-Hi-Res", "Nothing to export yet; run a scan first.")
            return
        path = filedialog.asksaveasfilename(
            title=f"Export {kind.upper()}", defaultextension=f".{kind}",
            initialfile=f"realhires-report.{kind}", filetypes=[(kind.upper(), f"*.{kind}")])
        if not path:
            return
        visible = [self.results[i] for i in self.tree.get_children()]
        rows = list(self.results.values()) if all_results or not visible else visible
        (write_csv if kind == "csv" else write_json)(rows, path)
        self.status_var.set(f"Exported {len(rows)} row(s) to {path}")

    def about(self) -> None:
        messagebox.showinfo(
            "About Real-Hi-Res",
            f"Real-Hi-Res {__version__}\n\nFinds fake hi-res audio (upsampled or 16-bit padded) and fake "
            "lossless files (FLAC/WAV/ALAC made from MP3/AAC) by analysing the audio itself.\n\n"
            "https://github.com/SU2H1/Real-Hi-Res")

    def quit_app(self) -> None:
        if self.scanner and self.scanning:
            self.scanner.stop_event.set()
        self.cache.close()
        self.root.destroy()


def main(argv=None) -> int:
    multiprocessing.freeze_support()
    args = list(sys.argv[1:] if argv is None else argv)
    try:
        root = tk.Tk()
    except tk.TclError as exc:  # e.g. started over SSH or on a server without a desktop
        print(f"Real-Hi-Res cannot open its window: {exc}\n"
              "The GUI needs a graphical desktop (X11, or Wayland with XWayland). "
              "On a server or over SSH use the command-line tool instead "
              "(realhires / realhires-cli).", file=sys.stderr)
        return 1
    root.title(f"Real-Hi-Res {__version__} — fake hi-res & fake lossless detector")
    root.geometry("1280x820")
    root.minsize(900, 600)
    icon = os.path.join(os.path.dirname(os.path.abspath(__file__)), "assets", "icon.png")
    try:
        root.iconphoto(True, tk.PhotoImage(file=icon))
    except (tk.TclError, OSError):
        pass  # icon is cosmetic only
    style = ttk.Style(root)
    if sys.platform.startswith("linux") and "clam" in style.theme_names():
        style.theme_use("clam")
        # clam paints the bar in the window colour, which makes it invisible.
        style.configure("Horizontal.TProgressbar", background="#4a90d9")
    style.configure("Treeview", rowheight=22)
    App(root, initial=[a for a in args if not a.startswith("-")])
    root.mainloop()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
