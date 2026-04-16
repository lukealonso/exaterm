#!/usr/bin/env bash
set -euo pipefail

PROFILE="release"
CARGO_FLAGS=(--release)

while [[ $# -gt 0 ]]; do
    case "$1" in
        --debug)
            PROFILE="debug"
            CARGO_FLAGS=()
            shift
            ;;
        --release)
            shift
            ;;
        *)
            echo "Usage: $0 [--debug|--release]" >&2
            exit 1
            ;;
    esac
done

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BUNDLE_DIR="$REPO_ROOT/target/package/Exaterm.app"
CONTENTS_DIR="$BUNDLE_DIR/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
PLIST_SRC="$REPO_ROOT/crates/exaterm-macos/resources/Info.plist"
BINARY_SRC="$REPO_ROOT/target/$PROFILE/exaterm-macos"

echo "Building exaterm-macos ($PROFILE)..."
(cd "$REPO_ROOT" && cargo build -p exaterm-macos "${CARGO_FLAGS[@]}")

if [[ ! -f "$BINARY_SRC" ]]; then
    echo "Error: binary not found at $BINARY_SRC" >&2
    exit 1
fi

if [[ ! -f "$PLIST_SRC" ]]; then
    echo "Error: Info.plist not found at $PLIST_SRC" >&2
    exit 1
fi

echo "Assembling Exaterm.app..."
rm -rf "$BUNDLE_DIR"
mkdir -p "$MACOS_DIR" "$CONTENTS_DIR/Resources"
printf 'APPL????' > "$CONTENTS_DIR/PkgInfo"

cp "$BINARY_SRC" "$MACOS_DIR/exaterm"
cp "$PLIST_SRC" "$CONTENTS_DIR/Info.plist"

echo "Package created at $BUNDLE_DIR"
