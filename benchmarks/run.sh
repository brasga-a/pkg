#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BENCH_DIR="${ROOT_DIR}/benchmarks"
RESULTS_ROOT="${BENCH_DIR}/results"
PKG_BIN="${ROOT_DIR}/target/release/pkg"
DEFAULT_DEB="${ROOT_DIR}/examples/hello_world/hello-world_1.0.0_amd64.deb"

RUNS=15
WARMUP=3
RESOURCE_RUNS=5
COMPARE_DPKG=false
COLLECT_RESOURCES=true
CACHE_MODE="warm"
CPU=""
TARGET_DEB=""
ORIGINAL_ARGS=("$@")

print_help() {
  cat <<'EOF'
Uso: ./benchmarks/run.sh [opções] [pacote.deb]

Opções:
  --compare-dpkg         Compara os caminhos end-to-end de pkg install e dpkg -i
  --runs <N>             Execuções medidas de latência (padrão: 15)
  --warmup <N>           Execuções de aquecimento (padrão: 3)
  --cache-mode <MODE>    warm (padrão) ou cold
  --cpu <LIST>           Fixa o comando medido com taskset (ex.: 2 ou 2,3)
  --resource-runs <N>    Amostras para CPU/RSS/I/O (padrão: 5)
  --no-resources         Não coleta métricas via /usr/bin/time -v
  --output-dir <DIR>     Raiz dos resultados (padrão: benchmarks/results)
  -h, --help             Exibe ajuda

Exemplos:
  ./benchmarks/run.sh
  ./benchmarks/run.sh ~/Downloads/discord.deb --compare-dpkg --runs 30 --warmup 5
  ./benchmarks/run.sh ~/Downloads/discord.deb --compare-dpkg --cache-mode cold --runs 10
  ./benchmarks/run.sh ~/Downloads/discord.deb --compare-dpkg --cpu 2
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --compare-dpkg) COMPARE_DPKG=true; shift ;;
    --runs) RUNS="$2"; shift 2 ;;
    --warmup) WARMUP="$2"; shift 2 ;;
    --cache-mode) CACHE_MODE="$2"; shift 2 ;;
    --cpu) CPU="$2"; shift 2 ;;
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

TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/pkg-bench.XXXXXX")"
PKG_BASELINE="${TMP_ROOT}/pkg-baseline"
PKG_DATA_DIR="${TMP_ROOT}/pkg-run"
DPKG_CLEANUP=false
PKG_NAME=""

cleanup() {
  set +e
  if [[ "$DPKG_CLEANUP" == true && -n "$PKG_NAME" ]]; then
    sudo -n dpkg --purge "$PKG_NAME" >/dev/null 2>&1 || true
  fi
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

if [[ "$COMPARE_DPKG" == true ]]; then
  for tool in dpkg dpkg-query dpkg-deb sudo; do
    command -v "$tool" >/dev/null || { echo "$tool é necessário para --compare-dpkg" >&2; exit 1; }
  done
  sudo -v
  if dpkg-query -W "$PKG_NAME" >/dev/null 2>&1; then
    echo "Recusando comparação: '$PKG_NAME' já existe no banco do dpkg." >&2
    echo "Use uma VM/host descartável ou remova o pacote conscientemente antes do benchmark." >&2
    exit 1
  fi
  DPKG_CLEANUP=true
fi

if [[ "$CACHE_MODE" == "cold" ]]; then
  command -v sudo >/dev/null || { echo "sudo é necessário para --cache-mode cold" >&2; exit 1; }
  sudo -v
fi

"$PKG_BIN" --data-dir "$PKG_BASELINE" list >/dev/null

SAFE_NAME="$(printf '%s' "$PKG_NAME" | tr -cs 'A-Za-z0-9._-' '_')"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)-${SAFE_NAME}-${CACHE_MODE}"
RUN_DIR="${RESULTS_ROOT}/${RUN_ID}"
RAW_DIR="${RUN_DIR}/raw"
mkdir -p "$RAW_DIR"

printf -v PKG_BIN_Q '%q' "$PKG_BIN"
printf -v TARGET_Q '%q' "$TARGET_DEB"
printf -v PKG_NAME_Q '%q' "$PKG_NAME"
printf -v PKG_BASELINE_Q '%q' "$PKG_BASELINE"
printf -v PKG_DATA_Q '%q' "$PKG_DATA_DIR"
printf -v CPU_Q '%q' "$CPU"

PKG_PREP="set -euo pipefail; rm -rf -- ${PKG_DATA_Q}; mkdir -p -- ${PKG_DATA_Q}; cp -a -- ${PKG_BASELINE_Q}/. ${PKG_DATA_Q}/"
DPKG_PREP="set -euo pipefail; sudo -n dpkg --purge ${PKG_NAME_Q} >/dev/null 2>&1 || true; if dpkg-query -W ${PKG_NAME_Q} >/dev/null 2>&1; then exit 1; fi"

if [[ "$CACHE_MODE" == "cold" ]]; then
  CACHE_RESET="; sync; sudo -n sh -c 'echo 3 > /proc/sys/vm/drop_caches'"
  PKG_PREP+="$CACHE_RESET"
  DPKG_PREP+="$CACHE_RESET"
fi

AFFINITY=""
if [[ -n "$CPU" ]]; then
  AFFINITY="taskset -c ${CPU_Q} "
fi

PKG_INSTALL_BASE="${PKG_BIN_Q} --data-dir ${PKG_DATA_Q} install ${TARGET_Q} >/dev/null"
PKG_INSTALL_CMD="${AFFINITY}${PKG_BIN_Q} --data-dir ${PKG_DATA_Q} install ${TARGET_Q} >/dev/null"
PKG_INFO_CMD="${AFFINITY}${PKG_BIN_Q} --data-dir ${PKG_DATA_Q} info ${TARGET_Q} >/dev/null"
PKG_REMOVE_CMD="${AFFINITY}${PKG_BIN_Q} --data-dir ${PKG_DATA_Q} remove ${PKG_NAME_Q} >/dev/null"
PKG_REMOVE_PREP="${PKG_PREP}; ${PKG_INSTALL_BASE}"
DPKG_INSTALL_CMD="${AFFINITY}sudo -n dpkg -i ${TARGET_Q} >/dev/null"

ENV_OUT="${RUN_DIR}/environment.txt"
{
  echo "pkg benchmark environment"
  echo "run_id: ${RUN_ID}"
  echo "utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "git_commit: $(git -C "$ROOT_DIR" rev-parse HEAD 2>/dev/null || echo unknown)"
  echo "target: ${TARGET_DEB}"
  echo "target_sha256: $(sha256sum "$TARGET_DEB" | awk '{print $1}')"
  echo "target_bytes: $(stat -c '%s' "$TARGET_DEB")"
  echo "package: ${PKG_NAME}"
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
  if [[ "$COMPARE_DPKG" == true ]]; then
    echo "dpkg: $(dpkg --version | head -n1)"
  fi
  printf 'invocation: ./benchmarks/run.sh'
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

JSON_OUT="${RUN_DIR}/results.json"
MD_OUT="${RUN_DIR}/results.md"
RESOURCE_JSON="${RUN_DIR}/resources.json"
RESOURCE_MD="${RUN_DIR}/resources.md"
CHART_OUT="${RUN_DIR}/benchmark_chart.png"

if [[ "$COMPARE_DPKG" == true ]]; then
  echo
  echo "ATENÇÃO: este é um comparativo end-to-end de caminhos de instalação."
  echo "pkg e dpkg têm semânticas diferentes; o ratio não é um benchmark de trabalho equivalente."

  run_latency "pkg install (${CACHE_MODE})" "$PKG_PREP" "$PKG_INSTALL_CMD" "${RAW_DIR}/pkg-install.json"
  run_latency "dpkg -i (${CACHE_MODE})" "$DPKG_PREP" "$DPKG_INSTALL_CMD" "${RAW_DIR}/dpkg-install.json"

  NOTE="End-to-end installation paths: pkg uses an isolated rootless store and default-denies maintainer scripts; dpkg owns native system state/paths and executes Debian lifecycle semantics. Relative ratios are not equivalent-work engine speedups."
  python3 "${BENCH_DIR}/merge_results.py" \
    --input "${RAW_DIR}/pkg-install.json" \
    --input "${RAW_DIR}/dpkg-install.json" \
    --name "pkg install (${CACHE_MODE})" \
    --name "dpkg -i (${CACHE_MODE})" \
    --comparable \
    --note "$NOTE" \
    --output-json "$JSON_OUT" \
    --output-markdown "$MD_OUT"

  if [[ "$COLLECT_RESOURCES" == true && -x /usr/bin/time ]]; then
    python3 "${BENCH_DIR}/resource_bench.py" \
      --runs "$RESOURCE_RUNS" \
      --name "pkg install (${CACHE_MODE})" \
      --prepare "$PKG_PREP" \
      -c "$PKG_INSTALL_CMD" \
      --name "dpkg -i (${CACHE_MODE})" \
      --prepare "$DPKG_PREP" \
      -c "$DPKG_INSTALL_CMD" \
      --export-json "$RESOURCE_JSON" \
      --export-markdown "$RESOURCE_MD"
  fi

  PLOT_TITLE="Installation path latency: pkg vs dpkg (${CACHE_MODE})"
else
  run_latency "pkg install (${CACHE_MODE})" "$PKG_PREP" "$PKG_INSTALL_CMD" "${RAW_DIR}/pkg-install.json"
  run_latency "pkg info (${CACHE_MODE})" "$PKG_PREP" "$PKG_INFO_CMD" "${RAW_DIR}/pkg-info.json"
  run_latency "pkg remove (${CACHE_MODE})" "$PKG_REMOVE_PREP" "$PKG_REMOVE_CMD" "${RAW_DIR}/pkg-remove.json"

  python3 "${BENCH_DIR}/merge_results.py" \
    --input "${RAW_DIR}/pkg-install.json" \
    --input "${RAW_DIR}/pkg-info.json" \
    --input "${RAW_DIR}/pkg-remove.json" \
    --name "pkg install (${CACHE_MODE})" \
    --name "pkg info (${CACHE_MODE})" \
    --name "pkg remove (${CACHE_MODE})" \
    --output-json "$JSON_OUT" \
    --output-markdown "$MD_OUT"

  if [[ "$COLLECT_RESOURCES" == true && -x /usr/bin/time ]]; then
    python3 "${BENCH_DIR}/resource_bench.py" \
      --runs "$RESOURCE_RUNS" \
      --name "pkg install (${CACHE_MODE})" \
      --prepare "$PKG_PREP" \
      -c "$PKG_INSTALL_CMD" \
      --name "pkg info (${CACHE_MODE})" \
      --prepare "$PKG_PREP" \
      -c "$PKG_INFO_CMD" \
      --name "pkg remove (${CACHE_MODE})" \
      --prepare "$PKG_REMOVE_PREP" \
      -c "$PKG_REMOVE_CMD" \
      --export-json "$RESOURCE_JSON" \
      --export-markdown "$RESOURCE_MD"
  fi

  PLOT_TITLE="pkg local .deb latency (${CACHE_MODE})"
fi

python3 "${BENCH_DIR}/plot.py" \
  --input "$JSON_OUT" \
  --output "$CHART_OUT" \
  --title "$PLOT_TITLE"

echo
echo "Benchmark concluído."
echo "Run: ${RUN_DIR}"
echo "  latency:     ${MD_OUT}"
echo "  raw JSON:    ${JSON_OUT}"
echo "  environment: ${ENV_OUT}"
if [[ -f "$RESOURCE_MD" ]]; then
  echo "  resources:   ${RESOURCE_MD}"
fi
