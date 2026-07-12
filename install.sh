#!/usr/bin/env bash
set -Eeuo pipefail

echo "Building Caret in release mode..."
cargo build --release

INSTALL_DIR="${HOME}/.local/bin"
mkdir -p "$INSTALL_DIR"
install -m 0755 target/release/caret "$INSTALL_DIR/caret"

echo
echo "Installed: $INSTALL_DIR/caret"

case ":${PATH}:" in
  *":${INSTALL_DIR}:"*) ;;
  *)
    echo
    echo "Add this directory to PATH:"
    echo "  export PATH=\"\$HOME/.local/bin:\$PATH\""
    ;;
esac

echo
echo "Run:"
echo "  caret README.md"
