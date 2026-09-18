#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${ROOT_DIR}/benchmarks/results/gates"
RUNS=10
WARMUP=2
ARTIFACT="${ROOT_DIR}/examples/hello_world/hello-world_1.0.0_amd64.deb"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --runs) RUNS="$2"; shift 2 ;;
    --warmup) WARMUP="$2"; shift 2 ;;
    --output-dir) OUT_DIR="$2"; shift 2 ;;
    *) ARTIFACT="$1"; shift ;;
  esac
done

mkdir -p "$OUT_DIR"
(cd "$ROOT_DIR" && cargo build --release --example bench_gates -p pkg-core)

BENCH_BIN="${ROOT_DIR}/target/release/examples/bench_gates"
JSON_OUT="${OUT_DIR}/results.json"
MD_OUT="${OUT_DIR}/results.md"

hyperfine \
  --warmup "$WARMUP" \
  --runs "$RUNS" \
  --export-json "$JSON_OUT" \
  --export-markdown "$MD_OUT" \
  --command-name repository-parsing "$BENCH_BIN repository" \
  --command-name solving "$BENCH_BIN solve '$ARTIFACT'" \
  --command-name extraction "$BENCH_BIN extraction '$ARTIFACT'" \
  --command-name activation "$BENCH_BIN activation '$ARTIFACT'" \
  --command-name recovery "$BENCH_BIN recovery"

python3 - "$JSON_OUT" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
data = json.loads(path.read_text())
for result in data.get("results", []):
    if result.get("exit_codes") and any(code != 0 for code in result["exit_codes"]):
        raise SystemExit(f"benchmark command failed: {result['command']}")
print(f"wrote {path}")
PY
