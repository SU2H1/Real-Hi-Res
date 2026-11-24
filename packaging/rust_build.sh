#!/usr/bin/env bash
# Build, test, verify and package one release asset of the Rust version.
# Used by the rust-* jobs of build.yml (Windows via Git Bash, macOS, Linux
# inside an AlmaLinux 8 container) and usable locally.
#
#   ASSET=RealHiRes-Rust-linux-x64 bash packaging/rust_build.sh
#
# Environment:
#   ASSET          name of the archive (required); the version from
#                  realhires/__init__.py is inserted after "RealHiRes-Rust-",
#                  so RealHiRes-Rust-windows-x64 becomes RealHiRes-Rust-1.12.0-windows-x64
#   PYTHON         python with numpy and pytest, for the comparison with the
#                  Python version (default: python)
#   MACOS_MIN      macOS only: minimum macOS the package must support (default 12.0)
#   MAX_GLIBC      Linux only: newest glibc the package may require (default 2.28)
#   SKIP_FETCH=1   use an FFmpeg already in build/ffmpeg
#   SKIP_TESTS=1   skip cargo test and the comparison with the Python version
# Output: out/<ASSET>.{zip,dmg,tar.gz}
set -euo pipefail
: "${ASSET:?set ASSET}"
PY="${PYTHON:-python}"
VERSION="$("$PY" -c "import re; print(re.search(r'__version__ = \"([^\"]+)\"', open('realhires/__init__.py').read()).group(1))")"
CARGO_VERSION="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' rust/Cargo.toml)"
if [ "$CARGO_VERSION" != "$VERSION" ]; then
  echo "ERROR: rust/Cargo.toml has version $CARGO_VERSION, realhires/__init__.py $VERSION" >&2; exit 1
fi
case "$ASSET" in
  RealHiRes-Rust-"$VERSION"-*) ;;
  RealHiRes-Rust-*) ASSET="RealHiRes-Rust-$VERSION-${ASSET#RealHiRes-Rust-}" ;;
  *) echo "ERROR: ASSET must start with RealHiRes-Rust-" >&2; exit 1 ;;
esac
echo "Building $ASSET (version $VERSION)"
OS="$(uname -s)"
MACOS_MIN="${MACOS_MIN:-12.0}"
MAX_GLIBC="${MAX_GLIBC:-2.28}"
[ "$OS" != Darwin ] || export MACOSX_DEPLOYMENT_TARGET="$MACOS_MIN"
EXE=""
case "$OS" in Linux|Darwin) ;; *) EXE=".exe" ;; esac
mkdir -p out

echo "::group::FFmpeg"
[ "${SKIP_FETCH:-0}" = 1 ] || "$PY" packaging/fetch_ffmpeg.py
export PATH="$PWD/build/ffmpeg:$PATH"
ffmpeg -hide_banner -version | sed -n 1p || true
echo "::endgroup::"

echo "::group::Build"
(cd rust && cargo build --release --locked)
BIN="rust/target/release"
echo "::endgroup::"

if [ "${SKIP_TESTS:-0}" != 1 ]; then
  echo "::group::Tests"
  # the core's unit tests and the desktop app's headless test
  (cd rust && cargo test --release --locked)
  # the same verdicts, texts and saved results as the Python version
  "$PY" -m pytest -q tests/test_rust_parity.py tests/test_detection.py
  echo "::endgroup::"
fi

echo "::group::Package"
rm -rf pkg && mkdir -p pkg
case "$OS" in
  Darwin)
    APP="pkg/RealHiRes-Rust/RealHiRes-Rust.app"
    MACOS="$APP/Contents/MacOS"
    mkdir -p "$MACOS" "$APP/Contents/Resources"
    cp "$BIN/realhires-gui" "$BIN/realhires" "$MACOS/"
    # FFmpeg next to the programs, where they look for it first
    cp build/ffmpeg/ffmpeg build/ffmpeg/ffprobe "$MACOS/"
    cp build/ffmpeg/FFMPEG-*.txt "$APP/Contents/Resources/"
    ICONSET=build/RealHiResRust.iconset
    rm -rf "$ICONSET" && mkdir -p "$ICONSET"
    for s in 16 32 128 256 512; do
      sips -z "$s" "$s" realhires/assets/icon.png --out "$ICONSET/icon_${s}x${s}.png" > /dev/null
      sips -z $((s * 2)) $((s * 2)) realhires/assets/icon.png --out "$ICONSET/icon_${s}x${s}@2x.png" > /dev/null
    done
    iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/RealHiRes.icns"
    cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleDisplayName</key><string>Real-Hi-Res Rust</string>
  <key>CFBundleExecutable</key><string>realhires-gui</string>
  <key>CFBundleIconFile</key><string>RealHiRes.icns</string>
  <key>CFBundleIdentifier</key><string>io.github.su2h1.realhires.rust</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>Real-Hi-Res Rust</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>$MACOS_MIN</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF
    # Ad-hoc signature: Apple Silicon refuses to run unsigned code.  The
    # helpers first, then the bundle, which seals them.
    codesign --force --sign - "$MACOS/ffmpeg" "$MACOS/ffprobe" "$MACOS/realhires"
    codesign --force --sign - "$APP"
    codesign --verify --deep --strict "$APP"
    "$PY" packaging/check_compat.py "$APP" --max-macos "$MACOS_MIN" --arch "$(uname -m)"
    CLI="$MACOS/realhires"
    ;;
  Linux)
    DIR=pkg/RealHiRes-Rust
    mkdir -p "$DIR/ffmpeg"
    cp "$BIN/realhires-gui" "$BIN/realhires" "$DIR/"
    cp build/ffmpeg/ffmpeg build/ffmpeg/ffprobe build/ffmpeg/FFMPEG-*.txt "$DIR/ffmpeg/"
    cp realhires/assets/icon.png "$DIR/realhires-rust.png"
    cp packaging/linux/install-rust.sh "$DIR/install.sh"
    cp packaging/linux/uninstall-rust.sh "$DIR/uninstall.sh"
    chmod +x "$DIR/install.sh" "$DIR/uninstall.sh"
    "$PY" packaging/check_compat.py "$DIR" --max-glibc "$MAX_GLIBC"
    CLI="$DIR/realhires"
    ;;
  *)
    DIR=pkg/RealHiRes-Rust
    mkdir -p "$DIR/ffmpeg"
    cp "$BIN/realhires-gui.exe" "$BIN/realhires.exe" "$DIR/"
    cp build/ffmpeg/ffmpeg.exe build/ffmpeg/ffprobe.exe build/ffmpeg/FFMPEG-*.txt "$DIR/ffmpeg/"
    CLI="$DIR/realhires.exe"
    for f in "$DIR/realhires-gui.exe" "$DIR/realhires.exe"; do
      echo "$(basename "$f"): $(wc -c < "$f") bytes, sha256 $(sha256sum "$f" | cut -d' ' -f1)"
    done
    ;;
esac
cp packaging/README-RUST.txt pkg/RealHiRes-Rust/README.txt
echo "::endgroup::"

echo "::group::Smoke test"
"$PY" packaging/smoke_test.py "$CLI"
echo "::endgroup::"

echo "::group::Archive"
case "$OS" in
  Darwin)
    ditto -c -k --keepParent pkg/RealHiRes-Rust "out/$ASSET.zip"
    ln -s /Applications pkg/RealHiRes-Rust/Applications
    for i in 1 2 3 4 5; do  # hdiutil is occasionally "Resource busy" on CI
      hdiutil create -volname "Real-Hi-Res Rust" -srcfolder pkg/RealHiRes-Rust -ov -format UDZO \
        "out/$ASSET.dmg" && break
      echo "hdiutil failed, retrying ($i)"; sleep $((i * 5))
    done
    [ -f "out/$ASSET.dmg" ]
    ;;
  Linux)
    tar -C pkg -czf "out/$ASSET.tar.gz" RealHiRes-Rust
    ;;
  *)
    (cd pkg && 7z a -tzip "../out/$ASSET.zip" RealHiRes-Rust > /dev/null)
    ;;
esac
ls -la out
echo "::endgroup::"
