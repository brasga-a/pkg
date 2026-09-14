#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BENCH_DIR="${ROOT_DIR}/benchmarks"
RESULTS_DIR="${BENCH_DIR}/results"
PKG_BIN="${ROOT_DIR}/target/release/pkg"
DEFAULT_DEB="${ROOT_DIR}/examples/hello_world/hello-world_1.0.0_amd64.deb"

# Default configuration
RUNS=15
WARMUP=3
COMPARE_DPKG=false
TARGET_DEB=""

print_help() {
  cat <<EOF
Uso: ./benchmarks/run.sh [opções] [caminho-para-pacote.deb]

Opções:
  --compare-dpkg       Compara o 'pkg install' contra 'sudo dpkg -i'
  --runs <N>           Número de execuções por teste (padrão: 15)
  --warmup <N>         Número de execuções de aquecimento (padrão: 3)
  --output-dir <DIR>   Diretório de saída para os resultados (padrão: benchmarks/results)
  -h, --help           Exibe esta mensagem de ajuda

Exemplos:
  ./benchmarks/run.sh
  ./benchmarks/run.sh /caminho/para/discord.deb
  ./benchmarks/run.sh /caminho/para/discord.deb --compare-dpkg
EOF
}

# Parse CLI arguments
while [[ $# -gt 0 ]]; do
  case "$1" in
    --compare-dpkg)
      COMPARE_DPKG=true
      shift
      ;;
    --runs)
      RUNS="$2"
      shift 2
      ;;
    --warmup)
      WARMUP="$2"
      shift 2
      ;;
    --output-dir)
      RESULTS_DIR="$2"
      shift 2
      ;;
    -h|--help)
      print_help
      exit 0
      ;;
    *)
      if [[ -z "$TARGET_DEB" ]]; then
        TARGET_DEB="$1"
        shift
      else
        echo "Argumento desconhecido: $1"
        print_help
        exit 1
      fi
      ;;
  esac
done

if [[ -z "$TARGET_DEB" ]]; then
  TARGET_DEB="$DEFAULT_DEB"
fi

mkdir -p "$RESULTS_DIR"

echo "============================================================"
echo " 🚀 pkg Benchmark Suite"
echo "============================================================"
echo "Pacote alvo: $TARGET_DEB"
echo "Repetições:  $RUNS (aquecimento: $WARMUP)"
echo "Resultados:  $RESULTS_DIR"
echo "============================================================"

# 1. Verificar se o binário release do pkg existe
if [[ ! -x "$PKG_BIN" ]]; then
  echo "⚠️  Binário de release não encontrado em $PKG_BIN."
  echo "Compilando agora em modo release (cargo build --release)..."
  (cd "$ROOT_DIR" && cargo build --release)
fi

# 2. Verificar se o pacote padrão existe; se não, construir
if [[ "$TARGET_DEB" == "$DEFAULT_DEB" && ! -f "$DEFAULT_DEB" ]]; then
  echo "Gerando pacote de exemplo hello-world..."
  (cd "${ROOT_DIR}/examples/hello_world" && ./build.sh)
fi

if [[ ! -f "$TARGET_DEB" ]]; then
  echo "Erro: Pacote $TARGET_DEB não encontrado."
  exit 1
fi

# 3. Detectar o nome do pacote via pkg info
PKG_NAME="$("$PKG_BIN" info "$TARGET_DEB" 2>/dev/null | grep -E "^\s*Name:" | awk '{print $2}' || echo "")"
if [[ -z "$PKG_NAME" ]]; then
  # Fallback: extrair nome do arquivo
  PKG_NAME="$(basename "$TARGET_DEB" | cut -d'_' -f1)"
fi
echo "Nome do pacote identificado: $PKG_NAME"

JSON_OUT="${RESULTS_DIR}/results.json"
MD_OUT="${RESULTS_DIR}/results.md"

# 4. Executar benchmark
if command -v hyperfine &>/dev/null; then
  echo "✅ hyperfine detectado. Executando benchmark com hyperfine..."

  if [[ "$COMPARE_DPKG" == true ]]; then
    echo "⚠️  Modo comparativo com dpkg ativado. Certifique-se de que o sudo esteja autenticado."
    sudo true

    hyperfine \
      --warmup "$WARMUP" \
      --runs "$RUNS" \
      --prepare "$PKG_BIN remove $PKG_NAME 2>/dev/null || true; sudo dpkg -r $PKG_NAME 2>/dev/null || true" \
      --export-json "$JSON_OUT" \
      --export-markdown "$MD_OUT" \
      "$PKG_BIN install $TARGET_DEB" \
      "sudo dpkg -i $TARGET_DEB"
  else
    hyperfine \
      --warmup "$WARMUP" \
      --runs "$RUNS" \
      --prepare "$PKG_BIN remove $PKG_NAME 2>/dev/null || true" \
      --export-json "$JSON_OUT" \
      --export-markdown "$MD_OUT" \
      "$PKG_BIN install $TARGET_DEB" \
      "$PKG_BIN info $TARGET_DEB"
  fi
else
  echo "ℹ️  hyperfine não instalado. Usando motor de benchmark integrado em Python..."
  echo "💡 Para instalar o hyperfine oficial: cargo install hyperfine"

  if [[ "$COMPARE_DPKG" == true ]]; then
    sudo true
    python3 "${BENCH_DIR}/fallback_bench.py" \
      --warmup "$WARMUP" \
      --runs "$RUNS" \
      --prepare "$PKG_BIN remove $PKG_NAME 2>/dev/null || true; sudo dpkg -r $PKG_NAME 2>/dev/null || true" \
      -c "$PKG_BIN install $TARGET_DEB" \
      --prepare "$PKG_BIN remove $PKG_NAME 2>/dev/null || true; sudo dpkg -r $PKG_NAME 2>/dev/null || true" \
      -c "sudo dpkg -i $TARGET_DEB" \
      --export-json "$JSON_OUT" \
      --export-markdown "$MD_OUT"
  else
    python3 "${BENCH_DIR}/fallback_bench.py" \
      --warmup "$WARMUP" \
      --runs "$RUNS" \
      --prepare "$PKG_BIN remove $PKG_NAME 2>/dev/null || true" \
      -c "$PKG_BIN install $TARGET_DEB" \
      -c "$PKG_BIN info $TARGET_DEB" \
      --export-json "$JSON_OUT" \
      --export-markdown "$MD_OUT"
  fi
fi

# 5. Gerar visualização ASCII e gráficos
echo ""
python3 "${BENCH_DIR}/plot.py" --input "$JSON_OUT" --output "${RESULTS_DIR}/benchmark_chart.png"

# Limpeza final
"$PKG_BIN" remove "$PKG_NAME" 2>/dev/null || true

echo "✨ Benchmark concluído com sucesso!"
echo "Relatórios salvos em: $RESULTS_DIR"
echo "  - Tabela Markdown: $MD_OUT"
echo "  - Dados brutos JSON: $JSON_OUT"
