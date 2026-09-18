#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUNS="${1:-100}"

if ! [[ "$RUNS" =~ ^[0-9]+$ ]] || [[ "$RUNS" -lt 1 ]]; then
  echo "usage: $0 [runs-per-target]" >&2
  exit 2
fi

cargo build --manifest-path "$ROOT_DIR/Cargo.toml" --release
"$ROOT_DIR/target/release/metadata" "-runs=$RUNS" "$ROOT_DIR/corpus/metadata"
"$ROOT_DIR/target/release/extraction" "-runs=$RUNS" "$ROOT_DIR/corpus/extraction"
