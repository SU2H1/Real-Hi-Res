#!/bin/sh
# Add Real-Hi-Res (Rust) to your desktop's application menu and put its
# command-line tool on your PATH as `realhires-rust`.  Installs for the
# current user only (no root needed); the app keeps running from this folder,
# so do not move or delete it afterwards (run uninstall.sh first, or simply
# run install.sh again after moving it).  It does not touch an installed
# Python version of Real-Hi-Res.
set -e
DIR=$(cd "$(dirname "$0")" && pwd)
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
BIN="$HOME/.local/bin"
mkdir -p "$DATA/applications" "$DATA/icons/hicolor/512x512/apps" "$BIN"

cp "$DIR/realhires-rust.png" "$DATA/icons/hicolor/512x512/apps/realhires-rust.png"
cat > "$DATA/applications/realhires-rust.desktop" <<EOD
[Desktop Entry]
Type=Application
Name=Real-Hi-Res (Rust)
GenericName=Fake hi-res audio detector
Comment=Find audio files that are not really high resolution
Exec="$DIR/realhires-gui" %F
Icon=realhires-rust
Terminal=false
Categories=AudioVideo;Audio;Utility;
Keywords=audio;hi-res;flac;upsampled;spectrum;
EOD
chmod +x "$DATA/applications/realhires-rust.desktop"
ln -sf "$DIR/realhires" "$BIN/realhires-rust"

command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$DATA/applications" >/dev/null 2>&1 || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$DATA/icons/hicolor" >/dev/null 2>&1 || true

echo "Real-Hi-Res (Rust) is now in your application menu."
echo "Command line: realhires-rust (linked in $BIN)"
case ":$PATH:" in
  *":$BIN:"*) ;;
  *) echo "Note: $BIN is not on your PATH; add it, or run $DIR/realhires directly." ;;
esac
