"""The 2.0 player's windows: Playback info (track, decoder, output, quality check)
and the equalizer (graphic and parametric)."""

from __future__ import annotations

import copy
import math
import tkinter as tk
from tkinter import ttk
from typing import TYPE_CHECKING, Dict, List, Tuple

from . import eq
from .player import fmt_db
from .player_theme import PillButton, Slider
from .player_pages import frame, label

if TYPE_CHECKING:  # pragma: no cover
    from .player_gui import PlayerApp


def _window(app: "PlayerApp", title: str) -> tk.Toplevel:
    w = tk.Toplevel(app.root)
    w.title(title)
    w.transient(app.root)
    app.theme.reg(w, bg="bg")
    w.bind("<Escape>", lambda e: w.event_generate("<<Close>>"))
    return w


class PlaybackInfoWindow:
    """Track, Decoder, Output and Quality check, each a two-column list; Copy."""

    def __init__(self, app: "PlayerApp"):
        self.app, t = app, app.theme
        self.win = _window(app, "Playback Info")
        self.win.protocol("WM_DELETE_WINDOW", self.close)
        self.win.bind("<<Close>>", lambda e: self.close())
        box = frame(self.win, t)
        box.pack(fill="both", expand=True, padx=24, pady=18)
        self.values: Dict[Tuple[str, str], tk.Label] = {}
        self.body = frame(box, t)
        self.body.pack(fill="both", expand=True)
        bottom = frame(box, t)
        bottom.pack(fill="x", pady=(14, 0))
        self.copied = label(bottom, t, "", "small", fg="text2")
        self.copied.pack(side="left")
        PillButton(bottom, t, "Close", kind="plain", command=self.close).pack(side="right")
        PillButton(bottom, t, "Copy", command=self.copy).pack(side="right", padx=(0, 8))
        self.sections: List[Tuple[str, List[Tuple[str, str]]]] = []
        self.refresh()
        self.win.minsize(560, 300)

    def refresh(self) -> None:
        sections = self.app.playback_info()
        layout = [(s, [k for k, _ in rows]) for s, rows in sections]
        if layout != [(s, [k for k, _ in rows]) for s, rows in self.sections]:
            self._build(sections)
        else:
            for s, rows in sections:
                for k, v in rows:
                    self.values[(s, k)].configure(text=v)
        self.sections = sections

    def _build(self, sections) -> None:
        t = self.app.theme
        for w in self.body.winfo_children():
            w.destroy()
        t.prune()
        self.values = {}
        row = 0
        for s, rows in sections:
            label(self.body, t, s, "h2").grid(row=row, column=0, columnspan=2, sticky="w",
                                               pady=(0 if row == 0 else 14, 4))
            row += 1
            for k, v in rows:
                label(self.body, t, k, "body", fg="text2").grid(row=row, column=0, sticky="nw", padx=(0, 18))
                val = label(self.body, t, v, "body", wraplength=520, justify="left")
                val.grid(row=row, column=1, sticky="w")
                self.values[(s, k)] = val
                row += 1
        self.body.columnconfigure(1, weight=1)

    def text(self) -> str:
        lines = []
        for s, rows in self.sections:
            lines.append(s)
            lines += [f"  {k}: {v}" for k, v in rows]
        return "\n".join(lines)

    def copy(self) -> None:
        self.win.clipboard_clear()
        self.win.clipboard_append(self.text())
        self.copied.configure(text="Copied to the clipboard.")

    def close(self) -> None:
        self.app.info_window = None
        self.win.destroy()


class EqualizerWindow:
    """Equalizer on, Prevent clipping, the response curve, and the Graphic and
    Parametric tabs.  Changes are heard at once; saved when the window closes."""

    CURVE_W, CURVE_H = 660, 150

    def __init__(self, app: "PlayerApp"):
        self.app, self.t = app, app.theme
        t = self.t
        self.state = eq.normalize(copy.deepcopy(app.state.eq))
        self.win = _window(app, "Equalizer")
        self.win.protocol("WM_DELETE_WINDOW", self.close)
        self.win.bind("<<Close>>", lambda e: self.close())
        box = frame(self.win, t)
        box.pack(fill="both", expand=True, padx=20, pady=16)
        top = frame(box, t)
        top.pack(fill="x")
        self.v_on = tk.BooleanVar(value=self.state["enabled"])
        self.v_clip = tk.BooleanVar(value=self.state["prevent_clipping"])
        ttk.Checkbutton(top, text="Equalizer on", variable=self.v_on, style="P.TCheckbutton",
                        command=self._toggles).pack(side="left")
        ttk.Checkbutton(top, text="Prevent clipping", variable=self.v_clip, style="P.TCheckbutton",
                        command=self._toggles).pack(side="left", padx=(18, 0))
        self.pre_note = label(top, t, "", "small", fg="text2")
        self.pre_note.pack(side="right")
        self.curve = tk.Canvas(box, width=self.CURVE_W, height=self.CURVE_H, highlightthickness=0, bd=0)
        t.reg(self.curve, bg="lcd")
        self.curve.pack(fill="x", pady=(10, 10))
        self.curve.bind("<Configure>", lambda e: self.draw_curve())
        self.nb = ttk.Notebook(box, style="P.TNotebook")
        self.nb.pack(fill="both", expand=True)
        self.graphic = frame(self.nb, t)
        self.param = frame(self.nb, t)
        self.nb.add(self.graphic, text="Graphic")
        self.nb.add(self.param, text="Parametric")
        self._build_graphic()
        self._build_param()
        bottom = frame(box, t)
        bottom.pack(fill="x", pady=(12, 0))
        PillButton(bottom, t, "Done", command=self.close).pack(side="right")
        PillButton(bottom, t, "Reset", kind="plain", command=self._reset).pack(side="right", padx=(0, 8))
        t.on_change(self.draw_curve, self.win)
        self.win.minsize(700, 560)
        self.draw_curve()

    # ---- graphic --------------------------------------------------------
    def _build_graphic(self) -> None:
        t, g = self.t, self.state["graphic"]
        head = frame(self.graphic, t)
        head.pack(fill="x", pady=(12, 6), padx=8)
        self.v_gon = tk.BooleanVar(value=g["enabled"])
        ttk.Checkbutton(head, text="Use the graphic equalizer", variable=self.v_gon, style="P.TCheckbutton",
                        command=self._toggles).pack(side="left")
        self.v_preset = tk.StringVar(value=g["preset"])
        cb = ttk.Combobox(head, textvariable=self.v_preset, values=eq.PRESET_NAMES + [eq.CUSTOM], state="readonly",
                          width=16, style="P.TCombobox", font=t.f("body"))
        cb.pack(side="right")
        cb.bind("<<ComboboxSelected>>", lambda e: self._preset(self.v_preset.get()))
        label(head, t, "Preset", "body", fg="text2").pack(side="right", padx=(0, 8))
        bands = frame(self.graphic, t)
        bands.pack(fill="both", expand=True, padx=8, pady=(4, 8))
        self.g_sliders: List[Slider] = []
        self.g_values: List[tk.Label] = []
        cols = [("Pre-amp", g["preamp"])] + list(zip(eq.GRAPHIC_LABELS, g["gains"]))
        for i, (name, value) in enumerate(cols):
            col = frame(bands, t)
            col.grid(row=0, column=i, padx=(0, 18) if i == 0 else 4, sticky="ns")
            val = label(col, t, self._db(value), "small", fg="text2", anchor="center", width=6)
            val.pack()
            s = Slider(col, t, -eq.GAIN_LIMIT_DB, eq.GAIN_LIMIT_DB, value,
                       command=lambda v, final, i=i: self._graphic_band(i, v, final), length=190,
                       orient="vertical", origin=0.0, reset=0.0, thickness=28)
            s.pack(pady=4)
            label(col, t, name, "small_b" if i == 0 else "small", fg="text" if i == 0 else "text2",
                  anchor="center").pack()
            if i:
                self.g_sliders.append(s)
                self.g_values.append(val)
            else:
                self.g_pre, self.g_pre_val = s, val
        label(self.graphic, t, "Drag a slider to boost or cut that band (up to 12 dB). Double-click a slider "
                               "to set it back to 0 dB.", "small", fg="text2").pack(anchor="w", padx=8, pady=(0, 8))

    def _db(self, v: float) -> str:
        return ("+" if v > 0.05 else "") + f"{v:.1f}"

    def _graphic_band(self, i: int, value: float, final: bool) -> None:
        value = round(value * 2) / 2  # half-dB steps
        g = self.state["graphic"]
        if i == 0:
            g["preamp"] = value
            self.g_pre_val.configure(text=self._db(value))
        else:
            g["gains"][i - 1] = value
            self.g_values[i - 1].configure(text=self._db(value))
            g["preset"] = eq.preset_name(g["gains"])
            self.v_preset.set(g["preset"])
        self._changed()

    def _preset(self, name: str) -> None:
        if name == eq.CUSTOM:
            return
        self.state = eq.apply_preset(self.state, name)
        self.v_preset.set(name)
        for s, v, lab in zip(self.g_sliders, self.state["graphic"]["gains"], self.g_values):
            s.set(v)
            lab.configure(text=self._db(v))
        self._changed()

    # ---- parametric -----------------------------------------------------
    def _build_param(self) -> None:
        t, p = self.t, self.state["parametric"]
        head = frame(self.param, t)
        head.pack(fill="x", pady=(12, 6), padx=8)
        self.v_pon = tk.BooleanVar(value=p["enabled"])
        ttk.Checkbutton(head, text="Use the parametric equalizer", variable=self.v_pon, style="P.TCheckbutton",
                        command=self._toggles).pack(side="left")
        pre = frame(self.param, t)
        pre.pack(fill="x", padx=8, pady=(2, 8))
        label(pre, t, "Pre-amp", "body_b").pack(side="left")
        self.p_pre_val = label(pre, t, f"{self._db(p['preamp'])} dB", "body", fg="text2", width=8)
        self.p_pre = Slider(pre, t, -eq.GAIN_LIMIT_DB, eq.GAIN_LIMIT_DB, p["preamp"], command=self._param_pre,
                            length=260, origin=0.0, reset=0.0)
        self.p_pre.pack(side="left", padx=(12, 6))
        self.p_pre_val.pack(side="left")
        self.rows = frame(self.param, t)
        self.rows.pack(fill="both", expand=True, padx=8)
        bottom = frame(self.param, t)
        bottom.pack(fill="x", padx=8, pady=(8, 10))
        self.add_btn = PillButton(bottom, t, "Add band", kind="plain", command=self._add_band)
        self.add_btn.pack(side="left")
        self._band_rows()

    def _band_rows(self) -> None:
        t = self.t
        for w in self.rows.winfo_children():
            w.destroy()
        t.prune()
        heads = ("On", "Type", "Frequency (Hz)", "Gain (dB)", "Q", "")
        for c, text in enumerate(heads):
            label(self.rows, t, text, "small_b", fg="text2").grid(row=0, column=c, sticky="w", padx=(0, 12),
                                                                   pady=(0, 4))
        type_labels = [eq.BAND_TYPE_LABELS[k] for k in eq.BAND_TYPES]
        for r, b in enumerate(self.state["parametric"]["bands"], start=1):
            on = tk.BooleanVar(value=b["on"])
            ttk.Checkbutton(self.rows, variable=on, style="P.TCheckbutton",
                            command=lambda r=r, v=on: self._band(r - 1, "on", v.get())).grid(row=r, column=0,
                                                                                            sticky="w")
            kind = tk.StringVar(value=eq.BAND_TYPE_LABELS[b["type"]])
            cb = ttk.Combobox(self.rows, textvariable=kind, values=type_labels, state="readonly", width=11,
                              style="P.TCombobox", font=t.f("body"))
            cb.grid(row=r, column=1, sticky="w", padx=(0, 12), pady=3)
            cb.bind("<<ComboboxSelected>>", lambda e, r=r, v=kind: self._band(
                r - 1, "type", eq.BAND_TYPES[type_labels.index(v.get())]))
            specs = (("freq", eq.MIN_FREQ, eq.MAX_FREQ, 10, "{:g}"),
                     ("gain", -eq.GAIN_LIMIT_DB, eq.GAIN_LIMIT_DB, 0.5, "{:g}"),
                     ("q", eq.MIN_Q, eq.MAX_Q, 0.1, "{:.2f}"))
            for c, (key, lo, hi, inc, fmt) in enumerate(specs, start=2):
                var = tk.StringVar(value=fmt.format(b[key]))
                sp = ttk.Spinbox(self.rows, textvariable=var, from_=lo, to=hi, increment=inc, width=9,
                                 style="P.TSpinbox", font=t.f("body"),
                                 command=lambda r=r, k=key, v=var: self._band_text(r - 1, k, v))
                sp.grid(row=r, column=c, sticky="w", padx=(0, 12), pady=3)
                for ev in ("<FocusOut>", "<Return>"):
                    sp.bind(ev, lambda e, r=r, k=key, v=var: self._band_text(r - 1, k, v))
                if key == "gain" and b["type"] not in eq.GAIN_TYPES:
                    sp.state(["disabled"])
            PillButton(self.rows, t, "Remove", kind="plain", height=28,
                       command=lambda r=r: self._remove_band(r - 1)).grid(row=r, column=5, sticky="w")
        self.add_btn.set(enabled=len(self.state["parametric"]["bands"]) < eq.MAX_PARAMETRIC_BANDS)

    def _band_text(self, i: int, key: str, var: tk.StringVar) -> None:
        try:
            value = float(var.get().replace(",", "."))
        except ValueError:
            return
        self._band(i, key, value)

    def _band(self, i: int, key: str, value) -> None:
        bands = self.state["parametric"]["bands"]
        if i >= len(bands):
            return
        old_type = bands[i]["type"]
        bands[i][key] = value
        bands[i] = eq.normalize_band(bands[i])
        if key == "type" and (old_type in eq.GAIN_TYPES) != (value in eq.GAIN_TYPES):
            self._band_rows()  # the gain field turns on or off
        self._changed()

    def _add_band(self) -> None:
        bands = self.state["parametric"]["bands"]
        if len(bands) < eq.MAX_PARAMETRIC_BANDS:
            bands.append({"type": "peak", "freq": 1000.0, "gain": 0.0, "q": 1.0, "on": True})
            self._band_rows()
            self._changed()

    def _remove_band(self, i: int) -> None:
        bands = self.state["parametric"]["bands"]
        if i < len(bands):
            bands.pop(i)
            self._band_rows()
            self._changed()

    def _param_pre(self, value: float, final: bool) -> None:
        value = round(value * 2) / 2
        self.state["parametric"]["preamp"] = value
        self.p_pre_val.configure(text=f"{self._db(value)} dB")
        self._changed()

    # ---- both -----------------------------------------------------------
    def _toggles(self) -> None:
        s = self.state
        s["enabled"], s["prevent_clipping"] = self.v_on.get(), self.v_clip.get()
        s["graphic"]["enabled"], s["parametric"]["enabled"] = self.v_gon.get(), self.v_pon.get()
        self._changed()

    def _reset(self) -> None:
        on = self.state["enabled"]
        self.state = eq.default_state()
        self.state["enabled"] = on
        g = self.state["graphic"]
        self.g_pre.set(0.0)
        self.g_pre_val.configure(text=self._db(0.0))
        for s, lab in zip(self.g_sliders, self.g_values):
            s.set(0.0)
            lab.configure(text=self._db(0.0))
        self.v_preset.set(g["preset"])
        self.v_gon.set(g["enabled"])
        self.v_pon.set(self.state["parametric"]["enabled"])
        self.v_clip.set(self.state["prevent_clipping"])
        self.p_pre.set(0.0)
        self.p_pre_val.configure(text=f"{self._db(0.0)} dB")
        self._band_rows()
        self._changed()

    def _changed(self) -> None:
        self.state = eq.normalize(self.state)
        self.app.set_eq(self.state)
        self.draw_curve()

    def draw_curve(self) -> None:
        t, c = self.t, self.curve
        c.delete("all")
        W, H = max(c.winfo_width(), 100), max(c.winfo_height(), 60)
        left, right, top, bottom = 44, 14, 12, 22
        pw, ph = W - left - right, H - top - bottom

        def xf(f: float) -> float:
            return left + pw * math.log10(f / 20.0) / 3.0

        def yf(db: float) -> float:
            return top + ph * (1 - (db + 12.0) / 24.0)

        for db in (-12, -6, 0, 6, 12):
            y = yf(db)
            c.create_line(left, y, W - right, y, fill=t.c("sep"), dash=() if db == 0 else (2, 3))
            c.create_text(left - 6, y, text=f"{db:+d}" if db else "0", anchor="e", fill=t.c("text2"),
                          font=t.f("small"))
        for f, text in ((20, "20"), (50, "50"), (100, "100"), (200, "200"), (500, "500"), (1000, "1K"),
                        (2000, "2K"), (5000, "5K"), (10000, "10K"), (20000, "20K")):
            x = xf(f)
            c.create_line(x, top, x, top + ph, fill=t.c("sep"), dash=(2, 3))
            c.create_text(x, H - 10, text=text, fill=t.c("text2"), font=t.f("small"))
        on = self.state["enabled"]
        pts = []
        for f, db in eq.response(self.state):
            pts += [xf(f), yf(max(-12.0, min(12.0, db)))]
        c.create_line(*pts, fill=t.c("accent") if on else t.c("text2"), width=2.5, smooth=True)
        pre = eq.preamp_db(self.state)
        self.pre_note.configure(text=(f"Pre-amp {fmt_db(pre)}" if on and abs(pre) >= 0.05 else
                                      "" if on else "The equalizer is off"))

    def close(self) -> None:
        self.app.save_eq(self.state)
        self.app.eq_window = None
        self.win.destroy()


__all__ = ["PlaybackInfoWindow", "EqualizerWindow"]
