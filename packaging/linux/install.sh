#!/bin/sh
# Add Universal Music Player to your desktop's application menu and put `realhires-cli`
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
Name=Universal Music Player
GenericName=Music player
Comment=Play your music and find files that are not really high resolution
Exec="$DIR/UniversalMusicPlayer" %F
Icon=realhires
Terminal=false
Categories=AudioVideo;Audio;Player;
Keywords=music;player;audio;hi-res;flac;lyrics;equalizer;upsampled;
EOF
chmod +x "$DATA/applications/realhires.desktop"
ln -sf "$DIR/realhires-cli" "$BIN/realhires-cli"

command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$DATA/applications" >/dev/null 2>&1 || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$DATA/icons/hicolor" >/dev/null 2>&1 || true

echo "Universal Music Player is now in your application menu."
echo "Command line: realhires-cli (linked in $BIN)"
case ":$PATH:" in
  *":$BIN:"*) ;;
  *) echo "Note: $BIN is not on your PATH; add it, or run $DIR/realhires-cli directly." ;;
esac
