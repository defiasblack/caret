#!/usr/bin/env bash
set -Eeuo pipefail

echo "Building Caret in release mode..."
cargo build --release --locked

INSTALL_DIR="${CARET_INSTALL_DIR:-${HOME}/.local/bin}"
mkdir -p "$INSTALL_DIR"
install -m 0755 target/release/caret "$INSTALL_DIR/caret"

if [[ -d runtime/pdfium ]]; then
  for library in runtime/pdfium/lib/libpdfium.so runtime/pdfium/lib/libpdfium.dylib; do
    if [[ -f "$library" ]]; then
      install -m 0755 "$library" "$INSTALL_DIR/$(basename "$library")"
    fi
  done
  if [[ -d runtime/pdfium/licenses ]]; then
    mkdir -p "$INSTALL_DIR/pdfium-licenses"
    cp -R runtime/pdfium/licenses/. "$INSTALL_DIR/pdfium-licenses/"
  fi
  install -m 0644 runtime/pdfium/LICENSE "$INSTALL_DIR/pdfium-LICENSE"
else
  echo "PDF support: run python3 scripts/setup_pdfium.py --install-dir \"$INSTALL_DIR\""
fi

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
