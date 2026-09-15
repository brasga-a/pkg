#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

cp -a "$ROOT/src/." "$WORK/"

OUTPUT="$ROOT/alpm-python-runtime-fixture-1.0.0-1-x86_64.pkg.tar.zst"
TAR_PATH="$WORK/alpm-python-runtime-fixture-1.0.0-1-x86_64.pkg.tar"

TZ=UTC tar \
  --sort=name \
  --mtime='UTC 2026-09-15' \
  --owner=0 --group=0 --numeric-owner \
  -C "$WORK" \
  -cf "$TAR_PATH" \
  .PKGINFO usr

zstd -q -19 -f "$TAR_PATH" -o "$OUTPUT"
sha256sum "$OUTPUT"
