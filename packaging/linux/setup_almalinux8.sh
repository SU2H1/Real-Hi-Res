#!/usr/bin/env bash
# Prepare an AlmaLinux 8 (glibc 2.28) container for building the Linux package.
# Building on an old glibc is what makes the package run on most distributions
# released since 2018 (RHEL/Rocky/Alma 8+, Debian 10+, Ubuntu 18.10+, Fedora 29+...).
# Creates a virtualenv in /opt/venv; use PYTHON=/opt/venv/bin/python afterwards.
set -euo pipefail
dnf -y -q install python3.12 python3.12-tkinter python3.12-pip python3.12-devel \
  binutils git tar xz gzip findutils which
python3.12 -m venv /opt/venv
/opt/venv/bin/python -m pip install -q --upgrade pip
/opt/venv/bin/python -m pip install -q numpy pyinstaller pillow pytest miniaudio
/opt/venv/bin/python -c "import tkinter, numpy, PyInstaller, miniaudio; print('tk', tkinter.TkVersion, 'numpy', numpy.__version__, 'pyinstaller', PyInstaller.__version__, 'miniaudio', miniaudio.__version__)"
ldd --version | sed -n 1p
