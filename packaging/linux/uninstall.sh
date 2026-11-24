#!/bin/sh
# Remove what install.sh added (menu entry, icon, realhires-cli link).
# This folder itself is left alone; delete it yourself if you no longer want it.
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
rm -f "$DATA/applications/realhires.desktop" \
      "$DATA/icons/hicolor/512x512/apps/realhires.png"
if [ -L "$HOME/.local/bin/realhires-cli" ]; then
  rm -f "$HOME/.local/bin/realhires-cli"
fi
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$DATA/applications" >/dev/null 2>&1 || true
echo "Universal Music Player menu entry and command link removed."
