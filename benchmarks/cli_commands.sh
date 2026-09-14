#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BENCH_DIR="${ROOT_DIR}/benchmarks"
RESULTS_ROOT="${BENCH_DIR}/results"
PKG_BIN="${ROOT_DIR}/target/release/pkg"
DEFAULT_DEB="${ROOT_DIR}/examples/hello_world/hello-world_1.0.0_amd64.deb"

RUNS=30
WARMUP=5
RESOURCE_RUNS=5
CACHE_MODE="warm"
COLLECT_RESOURCES=true
PROFILE="benchmark-cli"
CPU=""
TARGET_DEB=""
ORIGINAL_ARGS=("$@")

print_help() {
  cat <<'EOF'
Uso: ./benchmarks/cli_commands.sh [opções] [pacote.deb]

Benchmark dedicado aos caminhos de CLI que não são cobertos pelo benchmark
end-to-end principal:
  - install --dry-run
  - remove --dry-run
  - list em estado vazio
  - list em estado instalado
  - info por nome de pacote instalado

Opções:
  --runs <N>             Execuções medidas por comando (padrão: 30)
  --warmup <N>           Execuções de aquecimento (padrão: 5)
  --cache-mode <MODE>    warm (padrão) ou cold
  --cpu <LIST>           Fixa o comando medido com taskset (ex.: 2 ou 2,3)
  --profile <NAME>       Perfil usado pela fixture instalada (padrão: benchmark-cli)
  --resource-runs <N>    Amostras para CPU/RSS/I/O (padrão: 5)
  --no-resources         Não coleta métricas via /usr/bin/time -v
  --output-dir <DIR>     Raiz dos resultados (padrão: benchmarks/results)
  -h, --help             Exibe ajuda

Exemplos:
  ./benchmarks/cli_commands.sh
  ./benchmarks/cli_commands.sh ~/Downloads/discord.deb --runs 50 --warmup 10
  ./benchmarks/cli_commands.sh --cache-mode cold --cpu 2
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --runs) RUNS="$2"; shift 2 ;;
    --warmup) WARMUP="$2"; shift 2 ;;
    --cache-mode) CACHE_MODE="$2"; shift 2 ;;
    --cpu) CPU="$2"; shift 2 ;;
    --profile) PROFILE="$2"; shift 2 ;;
    --resource-runs) RESOURCE_RUNS="$2"; shift 2 ;;
    --no-resources) COLLECT_RESOURCES=false; shift ;;
    --output-dir) RESULTS_ROOT="$2"; shift 2 ;;
    -h|--help) print_help; exit 0 ;;
    *)
      if [[ -z "$TARGET_DEB" ]]; then
        TARGET_DEB="$1"
        shift
      else
        echo "Argumento desconhecido: $1" >&2
        exit 2
      fi
      ;;
  esac
done

[[ "$RUNS" =~ ^[1-9][0-9]*$ ]] || { echo "--runs deve ser >= 1" >&2; exit 2; }
[[ "$WARMUP" =~ ^[0-9]+$ ]] || { echo "--warmup deve ser >= 0" >&2; exit 2; }
[[ "$RESOURCE_RUNS" =~ ^[1-9][0-9]*$ ]] || { echo "--resource-runs deve ser >= 1" >&2; exit 2; }
[[ "$CACHE_MODE" == "warm" || "$CACHE_MODE" == "cold" ]] || {
  echo "--cache-mode deve ser warm ou cold" >&2
  exit 2
}
[[ -n "$PROFILE" ]] || { echo "--profile não pode ser vazio" >&2; exit 2; }

if [[ -n "$CPU" ]]; then
  [[ "$CPU" =~ ^[0-9,-]+$ ]] || { echo "--cpu aceita apenas listas/ranges numéricos" >&2; exit 2; }
  command -v taskset >/dev/null || { echo "taskset não encontrado" >&2; exit 1; }
fi

if [[ -z "$TARGET_DEB" ]]; then
  TARGET_DEB="$DEFAULT_DEB"
fi

if [[ ! -x "$PKG_BIN" ]]; then
  echo "Compilando pkg em release..."
  (cd "$ROOT_DIR" && cargo build --release)
fi

if [[ "$TARGET_DEB" == "$DEFAULT_DEB" && ! -f "$DEFAULT_DEB" ]]; then
  (cd "${ROOT_DIR}/examples/hello_world" && ./build.sh)
fi
[[ -f "$TARGET_DEB" ]] || { echo "Pacote não encontrado: $TARGET_DEB" >&2; exit 1; }
TARGET_DEB="$(realpath "$TARGET_DEB")"

if [[ "$CACHE_MODE" == "cold" ]]; then
  command -v sudo >/dev/null || { echo "sudo é necessário para --cache-mode cold" >&2; exit 1; }
  sudo -v
fi

TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/pkg-cli-bench.XXXXXX")"
EMPTY_BASELINE="${TMP_ROOT}/empty-baseline"
INSTALLED_BASELINE="${TMP_ROOT}/installed-baseline"
RUN_DATA_DIR="${TMP_ROOT}/run"
PKG_NAME=""

cleanup() {
  rm -rf -- "$TMP_ROOT"
}
trap cleanup EXIT INT TERM

if command -v dpkg-deb >/dev/null; then
  PKG_NAME="$(dpkg-deb -f "$TARGET_DEB" Package 2>/dev/null || true)"
fi
if [[ -z "$PKG_NAME" ]]; then
  PKG_NAME="$("$PKG_BIN" --data-dir "${TMP_ROOT}/probe" info "$TARGET_DEB" 2>/dev/null \
    | awk '/^[[:space:]]*Name:/ {print $2; exit}')"
fi
if [[ -z "$PKG_NAME" ]]; then
  PKG_NAME="$(basename "$TARGET_DEB" | cut -d_ -f1)"
fi

# Build two immutable logical baselines outside every timed window:
# 1. initialized but empty state;
# 2. the same state with the target package installed in PROFILE.
"$PKG_BIN" --data-dir "$EMPTY_BASELINE" --profile "$PROFILE" list >/dev/null
cp -a -- "$EMPTY_BASELINE" "$INSTALLED_BASELINE"
"$PKG_BIN" --data-dir "$INSTALLED_BASELINE" --profile "$PROFILE" install "$TARGET_DEB" >/dev/null

# Sanity checks: the installed baseline must expose the package through both list
# and info-by-name before it is used for benchmark preparation.
"$PKG_BIN" --data-dir "$INSTALLED_BASELINE" --profile "$PROFILE" list \
  | grep -Fq "$PKG_NAME" || {
    echo "Falha ao preparar baseline: '$PKG_NAME' não aparece em pkg list" >&2
    exit 1
  }
"$PKG_BIN" --data-dir "$INSTALLED_BASELINE" --profile "$PROFILE" info "$PKG_NAME" >/dev/null

SAFE_NAME="$(printf '%s' "$PKG_NAME" | tr -cs 'A-Za-z0-9._-' '_')"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)-${SAFE_NAME}-cli-${CACHE_MODE}"
RUN_DIR="${RESULTS_ROOT}/${RUN_ID}"
RAW_DIR="${RUN_DIR}/raw"
mkdir -p "$RAW_DIR"

printf -v PKG_BIN_Q '%q' "$PKG_BIN"
printf -v TARGET_Q '%q' "$TARGET_DEB"
printf -v PKG_NAME_Q '%q' "$PKG_NAME"
printf -v PROFILE_Q '%q' "$PROFILE"
printf -v EMPTY_BASELINE_Q '%q' "$EMPTY_BASELINE"
printf -v INSTALLED_BASELINE_Q '%q' "$INSTALLED_BASELINE"
printf -v RUN_DATA_Q '%q' "$RUN_DATA_DIR"
printf -v CPU_Q '%q' "$CPU"

EMPTY_PREP="set -euo pipefail; rm -rf -- ${RUN_DATA_Q}; mkdir -p -- ${RUN_DATA_Q}; cp -a -- ${EMPTY_BASELINE_Q}/. ${RUN_DATA_Q}/"
INSTALLED_PREP="set -euo pipefail; rm -rf -- ${RUN_DATA_Q}; mkdir -p -- ${RUN_DATA_Q}; cp -a -- ${INSTALLED_BASELINE_Q}/. ${RUN_DATA_Q}/"

if [[ "$CACHE_MODE" == "cold" ]]; then
  CACHE_RESET="; sync; sudo -n sh -c 'echo 3 > /proc/sys/vm/drop_caches'"
  EMPTY_PREP+="$CACHE_RESET"
  INSTALLED_PREP+="$CACHE_RESET"
fi

AFFINITY=""
if [[ -n "$CPU" ]]; then
  AFFINITY="taskset -c ${CPU_Q} "
fi

BASE="${AFFINITY}${PKG_BIN_Q} --data-dir ${RUN_DATA_Q} --profile ${PROFILE_Q}"
INSTALL_DRY_CMD="${BASE} install ${TARGET_Q} --dry-run >/dev/null"
REMOVE_DRY_CMD="${BASE} remove ${PKG_NAME_Q} --dry-run >/dev/null"
LIST_EMPTY_CMD="${BASE} list >/dev/null"
LIST_INSTALLED_CMD="${BASE} list >/dev/null"
INFO_INSTALLED_CMD="${BASE} info ${PKG_NAME_Q} >/dev/null"

ENV_OUT="${RUN_DIR}/environment.txt"
{
  echo "pkg CLI benchmark environment"
  echo "run_id: ${RUN_ID}"
  echo "utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "git_commit: $(git -C "$ROOT_DIR" rev-parse HEAD 2>/dev/null || echo unknown)"
  echo "target: ${TARGET_DEB}"
  echo "target_sha256: $(sha256sum "$TARGET_DEB" | awk '{print $1}')"
  echo "target_bytes: $(stat -c '%s' "$TARGET_DEB")"
  echo "package: ${PKG_NAME}"
  echo "profile: ${PROFILE}"
  echo "cache_mode: ${CACHE_MODE}"
  echo "runs: ${RUNS}"
  echo "warmup: ${WARMUP}"
  echo "resource_runs: ${RESOURCE_RUNS}"
  echo "cpu_affinity: ${CPU:-none}"
  echo "kernel: $(uname -srmo)"
  echo "os: $(grep '^PRETTY_NAME=' /etc/os-release 2>/dev/null | cut -d= -f2- | tr -d '\"' || true)"
  echo "cpu_model: $(lscpu 2>/dev/null | awk -F: '/Model name/ {sub(/^[[:space:]]+/, "", $2); print $2; exit}' || true)"
  echo "cpu_count: $(nproc 2>/dev/null || echo unknown)"
  if [[ -r /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor ]]; then
    echo "cpu_governor: $(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor)"
  fi
  echo "rustc: $(rustc --version 2>/dev/null || echo unavailable)"
  echo "cargo: $(cargo --version 2>/dev/null || echo unavailable)"
  echo "pkg: $("$PKG_BIN" --version 2>/dev/null || echo unavailable)"
  echo "hyperfine: $(hyperfine --version 2>/dev/null || echo unavailable)"
  printf 'invocation: ./benchmarks/cli_commands.sh'
  printf ' %q' "${ORIGINAL_ARGS[@]}"
  echo
} > "$ENV_OUT"

run_latency() {
  local label="$1"
  local prepare="$2"
  local command="$3"
  local output="$4"

  echo
  echo "==> ${label}"
  if command -v hyperfine >/dev/null; then
    hyperfine \
      --warmup "$WARMUP" \
      --runs "$RUNS" \
      --prepare "$prepare" \
      --export-json "$output" \
      "$command"
  else
    python3 "${BENCH_DIR}/fallback_bench.py" \
      --warmup "$WARMUP" \
      --runs "$RUNS" \
      --name "$label" \
      --prepare "$prepare" \
      -c "$command" \
      --export-json "$output"
  fi
}

run_latency "pkg install --dry-run (${CACHE_MODE})" "$EMPTY_PREP" "$INSTALL_DRY_CMD" "${RAW_DIR}/install-dry-run.json"
run_latency "pkg remove --dry-run (${CACHE_MODE})" "$INSTALLED_PREP" "$REMOVE_DRY_CMD" "${RAW_DIR}/remove-dry-run.json"
run_latency "pkg list empty (${CACHE_MODE})" "$EMPTY_PREP" "$LIST_EMPTY_CMD" "${RAW_DIR}/list-empty.json"
run_latency "pkg list installed (${CACHE_MODE})" "$INSTALLED_PREP" "$LIST_INSTALLED_CMD" "${RAW_DIR}/list-installed.json"
run_latency "pkg info installed (${CACHE_MODE})" "$INSTALLED_PREP" "$INFO_INSTALLED_CMD" "${RAW_DIR}/info-installed.json"

JSON_OUT="${RUN_DIR}/results.json"
MD_OUT="${RUN_DIR}/results.md"
RESOURCE_JSON="${RUN_DIR}/resources.json"
RESOURCE_MD="${RUN_DIR}/resources.md"
CHART_OUT="${RUN_DIR}/benchmark_chart.png"

NOTE="CLI operations have different semantics and state requirements. Compare each command against its own historical baseline; relative cross-command speedups are intentionally omitted."
python3 "${BENCH_DIR}/merge_results.py" \
  --input "${RAW_DIR}/install-dry-run.json" \
  --input "${RAW_DIR}/remove-dry-run.json" \
  --input "${RAW_DIR}/list-empty.json" \
  --input "${RAW_DIR}/list-installed.json" \
  --input "${RAW_DIR}/info-installed.json" \
  --name "pkg install --dry-run (${CACHE_MODE})" \
  --name "pkg remove --dry-run (${CACHE_MODE})" \
  --name "pkg list empty (${CACHE_MODE})" \
  --name "pkg list installed (${CACHE_MODE})" \
  --name "pkg info installed (${CACHE_MODE})" \
  --note "$NOTE" \
  --output-json "$JSON_OUT" \
  --output-markdown "$MD_OUT"

if [[ "$COLLECT_RESOURCES" == true && -x /usr/bin/time ]]; then
  python3 "${BENCH_DIR}/resource_bench.py" \
    --runs "$RESOURCE_RUNS" \
    --name "pkg install --dry-run (${CACHE_MODE})" \
    --prepare "$EMPTY_PREP" \
    -c "$INSTALL_DRY_CMD" \
    --name "pkg remove --dry-run (${CACHE_MODE})" \
    --prepare "$INSTALLED_PREP" \
    -c "$REMOVE_DRY_CMD" \
    --name "pkg list empty (${CACHE_MODE})" \
    --prepare "$EMPTY_PREP" \
    -c "$LIST_EMPTY_CMD" \
    --name "pkg list installed (${CACHE_MODE})" \
    --prepare "$INSTALLED_PREP" \
    -c "$LIST_INSTALLED_CMD" \
    --name "pkg info installed (${CACHE_MODE})" \
    --prepare "$INSTALLED_PREP" \
    -c "$INFO_INSTALLED_CMD" \
    --export-json "$RESOURCE_JSON" \
    --export-markdown "$RESOURCE_MD"
fi

python3 "${BENCH_DIR}/plot.py" \
  --input "$JSON_OUT" \
  --output "$CHART_OUT" \
  --title "pkg CLI command latency (${CACHE_MODE})"

echo
echo "Benchmark de CLI concluído."
echo "Run: ${RUN_DIR}"
echo "  latency:     ${MD_OUT}"
echo "  raw JSON:    ${JSON_OUT}"
echo "  environment: ${ENV_OUT}"
if [[ -f "$RESOURCE_MD" ]]; then
  echo "  resources:   ${RESOURCE_MD}"
fi
