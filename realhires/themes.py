"""Colour themes of the player (2.0).

A theme is a name, whether it is dark, and eight colours (`COLOR_KEYS`).
The player comes with the themes of `BUILTIN`; "Match the system" follows the
system's light or dark mode with "Light" and "Dark".  The user's own themes
are kept in `themes.json` next to the saved results (`store.data_dir()`, or
`$REALHIRES_THEMES`) and can be exported and imported as .json files.  The
`theme` setting holds the chosen name.  The Rust version reads the same file
and has the same built-in themes (`rust/crates/core/src/themes.rs`).
"""

from __future__ import annotations

import json
import os
import re
import tempfile
from typing import Dict, List, Optional

THEMES_ENV = "REALHIRES_THEMES"
FILE_NAME = "themes.json"
THEME_FILE_FORMAT = "real-hi-res-theme"
SYSTEM = "Match the system"
MAX_NAME = 80

COLOR_KEYS = ["background", "sidebar", "text", "secondary", "separator", "accent", "panel", "hover"]
COLOR_LABELS = {"background": "Background", "sidebar": "Sidebar", "text": "Text", "secondary": "Secondary text",
                "separator": "Lines", "accent": "Accent", "panel": "Playback panel", "hover": "Highlight"}


def _t(dark: bool, *colors: str) -> dict:
    return {"dark": dark, "colors": dict(zip(COLOR_KEYS, colors))}


BUILTIN: Dict[str, dict] = {
    "Light": _t(False, "#ffffff", "#f5f5f7", "#1d1d1f", "#6e6e73", "#d2d2d7", "#fa2d48", "#f2f2f4", "#e8e8ed"),
    "Dark": _t(True, "#1e1e1e", "#262628", "#f5f5f7", "#a1a1a6", "#3a3a3c", "#fa2d48", "#2c2c2e", "#333336"),
    "Midnight": _t(True, "#0f1221", "#161a2e", "#e8eaf6", "#9aa0c3", "#2a2f4a", "#7c8cff", "#1c2138", "#23294a"),
    "Graphite": _t(True, "#1b1b1d", "#232326", "#ececec", "#9a9a9f", "#38383c", "#ff9f0a", "#2a2a2d", "#313135"),
    "Forest": _t(True, "#121a15", "#18231c", "#e6efe8", "#9db3a3", "#2b3a30", "#4cc38a", "#1d2a22", "#24342a"),
    "Ocean": _t(False, "#f4f8fb", "#e8f0f6", "#102a43", "#52677c", "#c8d6e2", "#0a84ff", "#e9f1f7", "#dbe7f1"),
    "Sepia": _t(False, "#f6efe2", "#ece2cf", "#3b2f22", "#7a6a55", "#d9cbb1", "#c0673a", "#efe5d2", "#e3d6bd"),
    "Rose": _t(False, "#fff7f9", "#fbeaf0", "#3a1d2a", "#8a6574", "#efd3de", "#e0457b", "#f9e6ee", "#f3dbe5"),
}

_HEX = re.compile(r"^#[0-9a-f]{6}$")


class ThemeError(ValueError):
    """A theme name or file that cannot be used; the message says why."""


def default_path() -> str:
    from .store import data_dir

    return os.environ.get(THEMES_ENV) or os.path.join(data_dir(), FILE_NAME)


def clean_name(name) -> str:
    return " ".join(str(name or "").split())[:MAX_NAME]


def normalize(theme) -> Optional[dict]:
    """A valid theme from stored data, or None.  Missing or broken colours are
    taken from "Dark" or "Light"."""
    if not isinstance(theme, dict):
        return None
    name = clean_name(theme.get("name"))
    if not name:
        return None
    dark = bool(theme.get("dark", False))
    base = BUILTIN["Dark" if dark else "Light"]["colors"]
    given = theme.get("colors") if isinstance(theme.get("colors"), dict) else {}
    colors = {}
    for k in COLOR_KEYS:
        v = given.get(k)
        v = v.strip().lower() if isinstance(v, str) else ""
        colors[k] = v if _HEX.match(v) else base[k]
    return {"name": name, "dark": dark, "colors": colors}


def load_custom(path: Optional[str] = None) -> List[dict]:
    try:
        with open(path or default_path(), encoding="utf-8") as fh:
            data = json.load(fh)
    except (OSError, ValueError):
        return []
    out: List[dict] = []
    seen = set(n.casefold() for n in BUILTIN) | {SYSTEM.casefold()}
    for t in data.get("themes", []) if isinstance(data, dict) else []:
        n = normalize(t)
        if n is not None and n["name"].casefold() not in seen:
            seen.add(n["name"].casefold())
            out.append(n)
    return out


def save_custom(themes: List[dict], path: Optional[str] = None) -> str:
    path = path or default_path()
    folder = os.path.dirname(os.path.abspath(path))
    os.makedirs(folder, exist_ok=True)
    data = {"_version": 1, "themes": [normalize(t) for t in themes if normalize(t) is not None]}
    fd, tmp = tempfile.mkstemp(prefix=".themes-", suffix=".tmp", dir=folder)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as fh:
            json.dump(data, fh, ensure_ascii=False, indent=2, sort_keys=True)
            fh.write("\n")
        os.replace(tmp, path)
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise
    return path


def names(custom: List[dict]) -> List[str]:
    """Every theme name for the menu: Match the system, the built-in, then the user's."""
    return [SYSTEM] + list(BUILTIN) + [t["name"] for t in custom]


def resolve(name: str, system_dark: bool, custom: List[dict]) -> dict:
    """The theme to draw with; unknown names follow the system."""
    if name in BUILTIN:
        return dict(BUILTIN[name], name=name)
    for t in custom:
        if t["name"] == name:
            return t
    n = "Dark" if system_dark else "Light"
    return dict(BUILTIN[n], name=n)


def check_name(name: str, custom: List[dict], renaming: str = "") -> str:
    clean = clean_name(name)
    if not clean:
        raise ThemeError("Give the theme a name.")
    if clean.casefold() == SYSTEM.casefold() or clean.casefold() in {n.casefold() for n in BUILTIN}:
        raise ThemeError(f"“{clean}” is the name of a built-in theme; choose another one.")
    if any(t["name"].casefold() == clean.casefold() and t["name"] != renaming for t in custom):
        raise ThemeError(f"You already have a theme called “{clean}”.")
    return clean


def copy_name(base: str, custom: List[dict]) -> str:
    """"Midnight copy", "Midnight copy 2"... for a new theme made from `base`."""
    taken = {n.casefold() for n in names(custom)}
    name = clean_name(f"{base} copy")
    n = 2
    while name.casefold() in taken:
        name = clean_name(f"{base} copy {n}")
        n += 1
    return name


def export_theme(theme: dict) -> str:
    t = normalize(theme)
    if t is None:
        raise ThemeError("This theme cannot be exported.")
    data = {"format": THEME_FILE_FORMAT, "version": 1, "name": t["name"], "dark": t["dark"], "colors": t["colors"]}
    return json.dumps(data, ensure_ascii=False, indent=2, sort_keys=True) + "\n"


def import_theme(text: str, custom: List[dict]) -> dict:
    """A new custom theme from an exported file (a taken name gets " 2", " 3"...)."""
    try:
        data = json.loads(text.lstrip("﻿"))
    except ValueError:
        data = None
    t = normalize(data) if isinstance(data, dict) and isinstance(data.get("colors"), dict) else None
    if t is None:
        raise ThemeError("This file is not a theme of Universal Music Player.")
    taken = {n.casefold() for n in names(custom)}
    base = t["name"]
    n = 2
    while t["name"].casefold() in taken:
        t["name"] = clean_name(f"{base} {n}")
        n += 1
    return t
