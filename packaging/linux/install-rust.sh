#!/bin/sh
# Add Universal Music Player (Rust) to your desktop's application menu and put its
# command-line tool on your PATH as `realhires-rust`.  Installs for the
# current user only (no root needed); the app keeps running from this folder,
# so do not move or delete it afterwards (run uninstall.sh first, or simply
# run install.sh again after moving it).  It does not touch an installed
# Python version.
set -e
DIR=$(cd "$(dirname "$0")" && pwd)
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
BIN="$HOME/.local/bin"
mkdir -p "$DATA/applications" "$DATA/icons/hicolor/512x512/apps" "$BIN"

cp "$DIR/realhires-rust.png" "$DATA/icons/hicolor/512x512/apps/realhires-rust.png"
cat > "$DATA/applications/realhires-rust.desktop" <<EOD
[Desktop Entry]
Type=Application
Name=Universal Music Player (Rust)
GenericName=Music player
Comment=Play your music and find files that are not really high resolution
Exec="$DIR/universal-music-player" %F
Icon=realhires-rust
Terminal=false
Categories=AudioVideo;Audio;Player;
Keywords=music;player;audio;hi-res;flac;lyrics;equalizer;upsampled;
EOD
chmod +x "$DATA/applications/realhires-rust.desktop"
ln -sf "$DIR/realhires" "$BIN/realhires-rust"

command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$DATA/applications" >/dev/null 2>&1 || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$DATA/icons/hicolor" >/dev/null 2>&1 || true

echo "Universal Music Player (Rust) is now in your application menu."
echo "Command line: realhires-rust (linked in $BIN)"
case ":$PATH:" in
  *":$BIN:"*) ;;
  *) echo "Note: $BIN is not on your PATH; add it, or run $DIR/realhires directly." ;;
esac
