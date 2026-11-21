#!/usr/bin/env bash
set -e
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "Building ASCENSION +A toolchain (release mode)..."
cargo build --manifest-path "$DIR/rust/Cargo.toml" --release

INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
mkdir -p "$INSTALL_DIR"

cp -f "$DIR/rust/target/release/plus-a" "$INSTALL_DIR/plus-a"
cp -f "$DIR/rust/target/release/aa" "$INSTALL_DIR/aa"
ln -sf "$INSTALL_DIR/plus-a" "$INSTALL_DIR/+a"

echo "Successfully installed +A to $INSTALL_DIR/+a"
echo "You can now run: +a <source.pa>"
