#!/bin/sh
# Add Real-Hi-Res to your desktop's application menu and put `realhires-cli`
# on your PATH.  Installs for the current user only (no root needed); the app
# keeps running from this folder, so do not move or delete it afterwards
# (run uninstall.sh first, or simply run install.sh again after moving it).
set -e
DIR=$(cd "$(dirname "$0")" && pwd)
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
BIN="$HOME/.local/bin"
mkdir -p "$DATA/applications" "$DATA/icons/hicolor/512x512/apps" "$BIN"

cp "$DIR/realhires.png" "$DATA/icons/hicolor/512x512/apps/realhires.png"
cat > "$DATA/applications/realhires.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Real-Hi-Res
GenericName=Fake hi-res audio detector
Comment=Find audio files that are not really high resolution
Exec="$DIR/RealHiRes" %F
Icon=realhires
Terminal=false
Categories=AudioVideo;Audio;Utility;
Keywords=audio;hi-res;flac;upsampled;spectrum;
EOF
chmod +x "$DATA/applications/realhires.desktop"
ln -sf "$DIR/realhires-cli" "$BIN/realhires-cli"

command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$DATA/applications" >/dev/null 2>&1 || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$DATA/icons/hicolor" >/dev/null 2>&1 || true

echo "Real-Hi-Res is now in your application menu."
echo "Command line: realhires-cli (linked in $BIN)"
case ":$PATH:" in
  *":$BIN:"*) ;;
  *) echo "Note: $BIN is not on your PATH; add it, or run $DIR/realhires-cli directly." ;;
esac
