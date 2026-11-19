"""Desktop GUI (Tkinter, part of the Python standard library).

Pick folders or files, scan them in parallel, and scroll through the results.
By default only FAKE HI-RES and SUSPICIOUS files are listed; the other verdicts
can be switched on with the filter checkboxes.  Selecting a row shows the
reasons and the averaged spectrum of that file.

Run with ``realhires-gui`` (installed), ``python -m realhires.gui``, or the
``RealHiRes`` executable from a release package.
"""

from __future__ import annotations

import multiprocessing
import os
import queue
import subprocess
import sys
import threading
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
from .cli import AUDIO_EXTENSIONS, collect
from .probe import ToolMissingError, external_env, require_tools
from .report import write_csv, write_json
from .verdict import ERROR, FAKE, LOSSY, NOT_HIRES, REAL, SUSPECT, VERDICT_ORDER, FileResult, analyze_file

VERDICT_STYLE = {
    FAKE: ("Fake hi-res", "#c62828"),
    SUSPECT: ("Suspicious", "#b26a00"),
    REAL: ("Real hi-res", "#2e7d32"),
    NOT_HIRES: ("Not hi-res", "#1565c0"),
    LOSSY: ("Lossy format", "#6a1b9a"),
    ERROR: ("Error", "#616161"),
}
DEFAULT_SHOWN = {FAKE, SUSPECT}
QUICK_SECONDS = 90.0


def _work(path: str, max_seconds: Optional[float]) -> FileResult:
    """Runs in a worker process.  Must be module-level so it can be pickled."""
    return analyze_file(path, max_seconds=max_seconds, keep_plot=True)


def reveal_in_file_manager(path: str) -> None:
    env = external_env()  # do not leak the frozen app's library path
    if sys.platform.startswith("win"):
        subprocess.Popen(["explorer", "/select,", os.path.normpath(path)], env=env)
    elif sys.platform == "darwin":
        subprocess.Popen(["open", "-R", path], env=env)
    else:
        subprocess.Popen(["xdg-open", os.path.dirname(path) or "."], env=env)


class Scanner(threading.Thread):
    """Background thread: collects files, fans them out to worker processes
    and posts ("total"|"result"|"done"|"error", payload) messages to a queue."""

    def __init__(self, sources: List[str], jobs: int, max_seconds: Optional[float], out: queue.Queue):
        super().__init__(daemon=True)
        self.sources, self.jobs, self.max_seconds, self.out = sources, jobs, max_seconds, out
        self.stop_event = threading.Event()

    def run(self) -> None:
        try:
            files = collect(self.sources, recursive=True)
            self.out.put(("total", len(files)))
            if not files:
                self.out.put(("done", None))
                return
            ctx = multiprocessing.get_context("spawn")  # safe with a GUI thread running
            with ProcessPoolExecutor(max_workers=max(1, self.jobs), mp_context=ctx) as ex:
                futures = {ex.submit(_work, f, self.max_seconds): f for f in files}
                for fut in as_completed(futures):
                    if self.stop_event.is_set():
                        for other in futures:
                            other.cancel()
                        break
                    try:
                        self.out.put(("result", fut.result()))
                    except Exception as exc:  # noqa: BLE001 - e.g. a crashed worker
                        self.out.put(("result", FileResult(path=os.path.abspath(futures[fut]),
                                                           verdict=ERROR, reasons=[str(exc)])))
            self.out.put(("done", "stopped" if self.stop_event.is_set() else None))
        except Exception as exc:  # noqa: BLE001
            self.out.put(("error", str(exc)))


class SpectrumCanvas(tk.Canvas):
    """Draws the compact spectrum of the selected file."""

    def __init__(self, master, **kw):
        super().__init__(master, background="#ffffff", highlightthickness=0, **kw)
        self.result: Optional[FileResult] = None
        self.bind("<Configure>", lambda e: self.redraw())

    def show(self, result: Optional[FileResult]) -> None:
        self.result = result
        self.redraw()

    def redraw(self) -> None:
        self.delete("all")
        w, h = self.winfo_width(), self.winfo_height()
        r = self.result
        if w < 50 or h < 50:
            return
        if r is None or not r.plot or not r.plot.get("freqs"):
            msg = "Select a file to see its spectrum" if r is None else "No spectrum available"
            self.create_text(w / 2, h / 2, text=msg, fill="#888")
            return
        p = r.plot
        L, R, T, B = 52, 12, 26, 30
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
        color = VERDICT_STYLE.get(r.verdict, ("", "#1565c0"))[1]
        if p.get("bandwidth_hz"):
            x = X(p["bandwidth_hz"])
            self.create_line(x, T, x, T + ph, fill=color, dash=(6, 4), width=2)
            anchor, dx = ("e", -4) if x > L + pw - 130 else ("w", 4)
            self.create_text(x + dx, T + 10, text=f"bandwidth {p['bandwidth_hz'] / 1000:.1f} kHz",
                             anchor=anchor, fill=color)
        yf = Y(p["floor_db"])
        self.create_line(L, yf, L + pw, yf, fill="#8250df", dash=(4, 4))
        pts = []
        for f, v in zip(p["freqs"], p["db"]):
            pts += [X(f), Y(v)]
        if len(pts) >= 4:
            self.create_line(*pts, fill="#1f2328", width=1)
        self.create_rectangle(L, T, L + pw, T + ph, outline="#999")
        self.create_text(L, 12, anchor="w", font=("TkDefaultFont", 9, "bold"),
                         text=f"{os.path.basename(r.path)}  —  {r.verdict}", fill=color)


class App(ttk.Frame):
    COLUMNS = ("verdict", "claimed", "estimated", "bandwidth", "name", "folder")
    HEADINGS = {"verdict": "Verdict", "claimed": "Claimed", "estimated": "Real (est.)",
                "bandwidth": "Bandwidth", "name": "File", "folder": "Folder"}

    def __init__(self, root: tk.Tk, initial: Optional[List[str]] = None):
        super().__init__(root, padding=8)
        self.root = root
        self.sources: List[str] = []
        self.results: Dict[str, FileResult] = {}
        self.queue: queue.Queue = queue.Queue()
        self.scanner: Optional[Scanner] = None
        self.total = 0
        self.done = 0
        self.scanning = False
        self.sort_col, self.sort_rev = "verdict", False
        self.show_vars = {v: tk.BooleanVar(value=v in DEFAULT_SHOWN) for v in VERDICT_ORDER}
        self.search_var = tk.StringVar()
        self.quick_var = tk.BooleanVar(value=False)
        self.jobs_var = tk.IntVar(value=max(1, min(4, os.cpu_count() or 1)))
        self.status_var = tk.StringVar(value="Add a folder or files, then press Scan.")
        self.sources_var = tk.StringVar(value="Nothing selected yet.")
        self._build()
        self.grid(sticky="nsew")
        root.columnconfigure(0, weight=1)
        root.rowconfigure(0, weight=1)
        if initial:
            for p in initial:
                self._add_source(p)
            self.after(200, self.start_scan)
        self.after(100, self._poll)

    # ---------- layout ---------------------------------------------------
    def _build(self) -> None:
        self.columnconfigure(0, weight=1)
        bar = ttk.Frame(self)
        bar.grid(row=0, column=0, sticky="ew")
        ttk.Button(bar, text="Add Folder…", command=self.add_folder).pack(side="left")
        ttk.Button(bar, text="Add Files…", command=self.add_files).pack(side="left", padx=(4, 0))
        ttk.Button(bar, text="Clear", command=self.clear).pack(side="left", padx=(4, 12))
        ttk.Label(bar, text="Parallel jobs:").pack(side="left")
        ttk.Spinbox(bar, from_=1, to=max(32, os.cpu_count() or 1), width=4,
                    textvariable=self.jobs_var).pack(side="left", padx=(2, 12))
        ttk.Checkbutton(bar, text=f"Quick scan (first {QUICK_SECONDS:.0f} s)",
                        variable=self.quick_var).pack(side="left")
        self.stop_btn = ttk.Button(bar, text="Stop", command=self.stop_scan, state="disabled")
        self.stop_btn.pack(side="right")
        self.scan_btn = ttk.Button(bar, text="Scan", command=self.start_scan)
        self.scan_btn.pack(side="right", padx=(0, 4))

        ttk.Label(self, textvariable=self.sources_var, foreground="#555").grid(
            row=1, column=0, sticky="ew", pady=(6, 4))

        filt = ttk.Frame(self)
        filt.grid(row=2, column=0, sticky="ew", pady=(0, 4))
        ttk.Label(filt, text="Show:").pack(side="left")
        self.filter_checks: Dict[str, ttk.Checkbutton] = {}
        for v in VERDICT_ORDER:
            cb = ttk.Checkbutton(filt, text=VERDICT_STYLE[v][0], variable=self.show_vars[v],
                                 command=self.refresh)
            cb.pack(side="left", padx=(6, 0))
            self.filter_checks[v] = cb
        entry = ttk.Entry(filt, textvariable=self.search_var, width=28)
        entry.pack(side="right")
        ttk.Label(filt, text="Search:").pack(side="right", padx=(0, 4))
        self.search_var.trace_add("write", lambda *a: self.refresh())

        panes = ttk.PanedWindow(self, orient="vertical")
        panes.grid(row=3, column=0, sticky="nsew")
        self.rowconfigure(3, weight=1)

        table = ttk.Frame(panes)
        table.columnconfigure(0, weight=1)
        table.rowconfigure(0, weight=1)
        self.tree = ttk.Treeview(table, columns=self.COLUMNS, show="headings", selectmode="extended")
        widths = {"verdict": 110, "claimed": 80, "estimated": 90, "bandwidth": 85, "name": 300, "folder": 320}
        for c in self.COLUMNS:
            self.tree.heading(c, text=self.HEADINGS[c], command=lambda c=c: self.sort_by(c))
            self.tree.column(c, width=widths[c], stretch=c in ("name", "folder"),
                             anchor="w" if c in ("name", "folder", "verdict") else "center")
        for v, (_, color) in VERDICT_STYLE.items():
            self.tree.tag_configure(v, foreground=color)
        ys = ttk.Scrollbar(table, orient="vertical", command=self.tree.yview)
        xs = ttk.Scrollbar(table, orient="horizontal", command=self.tree.xview)
        self.tree.configure(yscrollcommand=ys.set, xscrollcommand=xs.set)
        self.tree.grid(row=0, column=0, sticky="nsew")
        ys.grid(row=0, column=1, sticky="ns")
        xs.grid(row=1, column=0, sticky="ew")
        self.tree.bind("<<TreeviewSelect>>", lambda e: self.show_details())
        self.tree.bind("<Double-1>", lambda e: self.reveal())
        self.tree.bind("<Button-3>", self._context_menu)
        self.tree.bind("<Button-2>", self._context_menu)  # macOS right click
        panes.add(table, weight=3)

        details = ttk.Frame(panes)
        details.columnconfigure(0, weight=1)
        details.columnconfigure(1, weight=1)
        details.rowconfigure(0, weight=1)
        txt_frame = ttk.Frame(details)
        txt_frame.grid(row=0, column=0, sticky="nsew", padx=(0, 6))
        txt_frame.rowconfigure(0, weight=1)
        txt_frame.columnconfigure(0, weight=1)
        self.text = tk.Text(txt_frame, wrap="word", height=10, relief="flat", padx=6, pady=6,
                            background="#fafafa", foreground="#1f2328")
        tys = ttk.Scrollbar(txt_frame, orient="vertical", command=self.text.yview)
        self.text.configure(yscrollcommand=tys.set, state="disabled")
        self.text.tag_configure("h", font=("TkDefaultFont", 10, "bold"))
        self.text.grid(row=0, column=0, sticky="nsew")
        tys.grid(row=0, column=1, sticky="ns")
        btns = ttk.Frame(txt_frame)
        btns.grid(row=1, column=0, columnspan=2, sticky="ew", pady=(4, 0))
        ttk.Button(btns, text="Show in Folder", command=self.reveal).pack(side="left")
        ttk.Button(btns, text="Copy Path", command=self.copy_path).pack(side="left", padx=4)
        ttk.Button(btns, text="Export CSV…", command=lambda: self.export("csv")).pack(side="right")
        ttk.Button(btns, text="Export JSON…", command=lambda: self.export("json")).pack(side="right", padx=4)
        self.canvas = SpectrumCanvas(details, height=240)
        self.canvas.grid(row=0, column=1, sticky="nsew")
        panes.add(details, weight=2)

        status = ttk.Frame(self)
        status.grid(row=4, column=0, sticky="ew", pady=(6, 0))
        status.columnconfigure(1, weight=1)
        self.progress = ttk.Progressbar(status, length=220, mode="determinate")
        self.progress.grid(row=0, column=0, padx=(0, 8))
        ttk.Label(status, textvariable=self.status_var).grid(row=0, column=1, sticky="w")

        self.menu = tk.Menu(self.root, tearoff=False)
        self.menu.add_command(label="Show in Folder", command=self.reveal)
        self.menu.add_command(label="Copy Path", command=self.copy_path)

    # ---------- sources --------------------------------------------------
    def _add_source(self, path: str) -> None:
        if path and path not in self.sources:
            self.sources.append(path)
        n = len(self.sources)
        shown = "; ".join(self.sources[:3]) + (f"; … (+{n - 3} more)" if n > 3 else "")
        self.sources_var.set(f"{n} source(s): {shown}" if n else "Nothing selected yet.")

    def add_folder(self) -> None:
        d = filedialog.askdirectory(title="Choose a music folder")
        if d:
            self._add_source(d)

    def add_files(self) -> None:
        pattern = " ".join(f"*{e}" for e in sorted(AUDIO_EXTENSIONS))
        files = filedialog.askopenfilenames(title="Choose audio files",
                                            filetypes=[("Audio files", pattern), ("All files", "*")])
        for f in files:
            self._add_source(f)

    def clear(self) -> None:
        if self.scanning:
            return
        self.sources.clear()
        self.results.clear()
        self._add_source("")
        self.refresh()
        self.canvas.show(None)
        self._set_text([])
        self.progress["value"] = 0
        self.status_var.set("Add a folder or files, then press Scan.")

    # ---------- scanning -------------------------------------------------
    def start_scan(self) -> None:
        if self.scanning:
            return
        if not self.sources:
            messagebox.showinfo("Real-Hi-Res", "Add a folder or some files first.")
            return
        try:
            require_tools()
        except ToolMissingError as exc:
            messagebox.showerror("FFmpeg not found", str(exc))
            return
        try:
            jobs = int(self.jobs_var.get())
        except (tk.TclError, ValueError):
            jobs = 1
        self.results.clear()
        self.refresh()
        self.total = self.done = 0
        self.scanning = True
        self.progress.configure(mode="indeterminate")
        self.progress.start(12)
        self.status_var.set("Looking for audio files…")
        self.scan_btn.configure(state="disabled")
        self.stop_btn.configure(state="normal")
        self.scanner = Scanner(list(self.sources), jobs,
                               QUICK_SECONDS if self.quick_var.get() else None, self.queue)
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
                kind, payload = self.queue.get_nowait()
                if kind == "total":
                    self.total = payload
                    self.progress.stop()
                    self.progress.configure(mode="determinate", maximum=max(payload, 1), value=0)
                    if payload == 0:
                        self.status_var.set("No audio files found in the selected sources.")
                elif kind == "result":
                    self.done += 1
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
        self.after(100, self._poll)

    def _finish(self, stopped: bool) -> None:
        self.scanning = False
        self.progress.stop()
        self.progress.configure(mode="determinate", maximum=max(self.total, 1), value=self.done)
        self.scan_btn.configure(state="normal")
        self.stop_btn.configure(state="disabled")
        self._update_status(final=True, stopped=stopped)
        self.refresh()

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
        self.status_var.set(f"{head}: {self.done}/{self.total} files  ·  {c[FAKE]} fake, "
                            f"{c[SUSPECT]} suspicious, {c[REAL]} real hi-res")

    # ---------- table ----------------------------------------------------
    def _matches(self, r: FileResult) -> bool:
        if not self.show_vars.get(r.verdict, tk.BooleanVar(value=True)).get():
            return False
        q = self.search_var.get().strip().lower()
        return not q or q in r.path.lower()

    def _values(self, r: FileResult):
        bw = f"{r.bandwidth_hz / 1000:.1f} kHz" if r.bandwidth_hz else "-"
        return (VERDICT_STYLE[r.verdict][0], r.claimed, r.estimated, bw,
                os.path.basename(r.path), os.path.dirname(r.path))

    def _insert(self, r: FileResult) -> None:
        if self._matches(r) and not self.tree.exists(r.path):
            self.tree.insert("", "end", iid=r.path, values=self._values(r), tags=(r.verdict,))

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
        keep = [i for i in selected if self.tree.exists(i)]
        if keep:
            self.tree.selection_set(keep)
        for c in self.COLUMNS:
            arrow = (" ▼" if self.sort_rev else " ▲") if c == self.sort_col else ""
            self.tree.heading(c, text=self.HEADINGS[c] + arrow)
        if self.results:
            self._update_status(final=not self.scanning)

    # ---------- details --------------------------------------------------
    def _current(self) -> Optional[FileResult]:
        sel = self.tree.selection()
        return self.results.get(sel[0]) if sel else None

    def _set_text(self, parts) -> None:
        self.text.configure(state="normal")
        self.text.delete("1.0", "end")
        for chunk, tag in parts:
            self.text.insert("end", chunk, tag)
        self.text.configure(state="disabled")

    def show_details(self) -> None:
        r = self._current()
        self.canvas.show(r)
        if r is None:
            self._set_text([])
            return
        parts = [(f"{r.verdict}\n", "h"), (f"{r.path}\n\n", None)]
        parts.append(("Why\n", "h"))
        parts += [(f"• {x}\n", None) for x in r.reasons]
        if r.notes:
            parts += [(f"• {x}\n", None) for x in r.notes]
        parts.append(("\nMeasurements\n", "h"))
        dur = f"{r.duration:.0f} s" if r.duration else "?"
        lines = [
            f"Codec: {r.codec}   Channels: {r.channels}   Duration: {dur}",
            f"Claimed: {r.claimed}   Estimated real: {r.estimated}",
            f"Significant bits: {r.effective_bits if r.effective_bits is not None else '?'}",
            f"Bandwidth: {r.bandwidth_hz / 1000:.1f} kHz" if r.bandwidth_hz else "Bandwidth: -",
        ]
        if r.cliffs_db:
            lines.append("Drop at source Nyquists: " + ", ".join(
                f"{k / 2000:g} kHz {v:.0f} dB" for k, v in r.cliffs_db.items() if v == v))
        parts += [(f"{x}\n", None) for x in lines]
        self._set_text(parts)

    def reveal(self) -> None:
        r = self._current()
        if r:
            try:
                reveal_in_file_manager(r.path)
            except OSError as exc:
                messagebox.showerror("Real-Hi-Res", f"Could not open the folder: {exc}")

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

    def export(self, kind: str) -> None:
        if not self.results:
            messagebox.showinfo("Real-Hi-Res", "Nothing to export yet; run a scan first.")
            return
        path = filedialog.asksaveasfilename(
            title=f"Export {kind.upper()}", defaultextension=f".{kind}",
            initialfile=f"realhires-report.{kind}", filetypes=[(kind.upper(), f"*.{kind}")])
        if not path:
            return
        visible = [self.results[i] for i in self.tree.get_children()]
        rows = visible if visible else list(self.results.values())
        (write_csv if kind == "csv" else write_json)(rows, path)
        self.status_var.set(f"Exported {len(rows)} row(s) (the files currently listed) to {path}")


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
    root.title(f"Real-Hi-Res {__version__} — fake hi-res audio detector")
    root.geometry("1180x760")
    root.minsize(820, 520)
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
