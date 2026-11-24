#!/usr/bin/env bash
# Make sure the stable Rust toolchain is installed (CI runners and the
# AlmaLinux 8 build container).  Prints the directory to add to PATH.
set -euo pipefail
if ! command -v rustup >/dev/null 2>&1 && [ ! -x "$HOME/.cargo/bin/rustup" ]; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal >&2
fi
export PATH="$HOME/.cargo/bin:$PATH"
rustup toolchain install stable --profile minimal >&2
rustup default stable >&2
rustc --version >&2
echo "$HOME/.cargo/bin"
