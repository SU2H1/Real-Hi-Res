#!/usr/bin/env bash
# Build, verify and package one release asset.  Used by every CI job (Windows
# via Git Bash, macOS, Linux inside an AlmaLinux 8 container) and usable locally.
#
#   ASSET=RealHiRes-linux-x64 bash packaging/ci_build.sh
#
# Environment:
#   ASSET          name of the archive to produce (required)
#   PYTHON         python to use (default: python)
#   MACOS_MIN      macOS only: minimum macOS the package must support (default 12.0)
#   MAX_GLIBC      Linux only: newest glibc the package may require (default 2.28)
#   SKIP_FETCH=1   use an FFmpeg already in build/ffmpeg (or none)
#   SKIP_TESTS=1   skip pytest
# Output: out/<ASSET>.{zip,dmg,tar.gz}
set -euo pipefail
: "${ASSET:?set ASSET}"
PY="${PYTHON:-python}"
export REALHIRES_MACOS_MIN="${MACOS_MIN:-12.0}"
MAX_GLIBC="${MAX_GLIBC:-2.28}"
OS="$(uname -s)"
mkdir -p out

echo "::group::FFmpeg"
[ "${SKIP_FETCH:-0}" = 1 ] || "$PY" packaging/fetch_ffmpeg.py
export PATH="$PWD/build/ffmpeg:$PATH"
ffmpeg -hide_banner -version | sed -n 1p || true
echo "::endgroup::"

if [ "${SKIP_TESTS:-0}" != 1 ]; then
  echo "::group::Tests"
  "$PY" -m pytest -q
  echo "::endgroup::"
fi

echo "::group::PyInstaller"
"$PY" -m PyInstaller --noconfirm packaging/realhires.spec
echo "::endgroup::"

case "$OS" in
  Darwin)
    APP=dist/RealHiRes.app
    CLI="$APP/Contents/MacOS/realhires-cli"
    "$PY" packaging/check_compat.py "$APP" --max-macos "$REALHIRES_MACOS_MIN" --arch "$(uname -m)"
    # Apple Silicon refuses to run unsigned code, so the ad-hoc signature that
    # PyInstaller applies must be intact there.  Intel Macs run unsigned code.
    if codesign --verify --deep --strict "$APP"; then
      echo "Code signature (ad-hoc) is valid."
    elif [ "$(uname -m)" = arm64 ]; then
      echo "ERROR: invalid code signature on an arm64 build" >&2; exit 1
    else
      echo "Note: app is not signed (fine on Intel Macs)."
    fi
    ;;
  Linux)
    CLI=dist/RealHiRes/realhires-cli
    "$PY" packaging/check_compat.py dist/RealHiRes --max-glibc "$MAX_GLIBC"
    ;;
  *)
    CLI=dist/RealHiRes/realhires-cli.exe
    "$PY" packaging/check_windows.py dist/RealHiRes
    ;;
esac

echo "::group::Smoke test"
"$PY" packaging/smoke_test.py "$CLI"
echo "::endgroup::"

echo "::group::Package"
case "$OS" in
  Darwin)
    rm -rf pkg && mkdir -p pkg/RealHiRes
    ditto "$APP" pkg/RealHiRes/RealHiRes.app
    cp packaging/README-RELEASE.txt pkg/RealHiRes/README.txt
    # Zip (keeps symlinks and permissions inside the .app)
    ditto -c -k --keepParent pkg/RealHiRes "out/$ASSET.zip"
    # Disk image with the usual drag-to-Applications shortcut
    ln -s /Applications pkg/RealHiRes/Applications
    for i in 1 2 3 4 5; do  # hdiutil is occasionally "Resource busy" on CI
      hdiutil create -volname "Real-Hi-Res" -srcfolder pkg/RealHiRes -ov -format UDZO \
        "out/$ASSET.dmg" && break
      echo "hdiutil failed, retrying ($i)"; sleep $((i * 5))
    done
    [ -f "out/$ASSET.dmg" ]
    ;;
  Linux)
    cp packaging/README-RELEASE.txt dist/RealHiRes/README.txt
    cp packaging/linux/install.sh packaging/linux/uninstall.sh dist/RealHiRes/
    cp realhires/assets/icon.png dist/RealHiRes/realhires.png
    chmod +x dist/RealHiRes/install.sh dist/RealHiRes/uninstall.sh
    tar -C dist -czf "out/$ASSET.tar.gz" RealHiRes
    ;;
  *)
    cp packaging/README-RELEASE.txt dist/RealHiRes/README.txt
    (cd dist && 7z a -tzip "../out/$ASSET.zip" RealHiRes > /dev/null)
    ;;
esac
ls -la out
echo "::endgroup::"
