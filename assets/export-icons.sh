#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC="$ROOT/exaterm-icon.svg"
OUT="$ROOT/generated"

mkdir -p "$OUT"

for size in 16 24 32 48 64 96 128 256 512; do
  rsvg-convert -w "$size" -h "$size" "$SRC" -o "$OUT/exaterm-icon-${size}.png"
done

echo "Exported icon sizes to $OUT"
