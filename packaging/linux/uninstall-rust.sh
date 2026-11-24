#!/bin/sh
# Remove what install.sh added (menu entry, icon, realhires-rust link).
# This folder itself is left alone; delete it yourself if you no longer want it.
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
rm -f "$DATA/applications/realhires-rust.desktop" \
      "$DATA/icons/hicolor/512x512/apps/realhires-rust.png"
if [ -L "$HOME/.local/bin/realhires-rust" ]; then
  rm -f "$HOME/.local/bin/realhires-rust"
fi
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$DATA/applications" >/dev/null 2>&1 || true
echo "Universal Music Player (Rust) menu entry and command link removed."
