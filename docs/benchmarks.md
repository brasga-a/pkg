# Benchmarks e Performance

Os benchmarks medem o custo do caminho rootless do `pkg` mantendo previsibilidade
e isolamento. Eles não constituem uma promessa de desempenho entre hosts ou
contra gerenciadores nativos.

---

## ⚡ Por que o `pkg` é tão rápido?

O baseline versionado usa a fixture local `hello-world`; os números atuais e o
ambiente estão em [benchmark-baseline-2026-09-17.md](benchmark-baseline-2026-09-17.md).
Comparações com `apt`, `dnf` ou `pacman` exigem a mesma fixture, host e política
de cache e ainda não fazem parte do gate publicado.

### Comparativo Arquitetural: `pkg` vs. Gerenciadores Nativos

| Fator de Performance | Gerenciador Tradicional (`apt` / `dpkg`) | `pkg` (Rust 2024 Kernel) |
|---|---|---|
| **Permissões / Locks** | Exige `sudo` e bloqueia travas globais do sistema (`/var/lib/dpkg/lock-frontend`). | Execução rootless isolada com `flock` consultivo no `$HOME`. Zero espera por outros processos do SO. |
| **Scripts de Mantenedor** | Executa scripts Bash arbitrários (`preinst`, `postinst`), recriando caches de fontes, man-pages e triggers do sistema. | **Default-Deny:** Não executa scripts desnecessários no host. Apenas materializa os binários da aplicação. |
| **Operações de Disco** | Descompacta milhares de arquivos diretamente espalhados na raiz (`/usr/share/`, `/usr/bin/`, `/etc/`). | Descompacta no `staging/` e promove atomicamente para a `store/` isolada com um único `rename()` de sistema de arquivos. |
| **Ativação de Comandos** | Registro pesado em bases de dados relacionais complexas do sistema. | Criação atômica de symlinks direcionados em `profiles/default/bin/`. |
| **Descompactação** | Forks sucessivos de processos `dpkg-deb` e `tar`. | Leitura em streaming via memória em Rust puro com aceleração por hardware. |

---

## 🛠️ Suíte de Benchmarks Automatizada

O repositório inclui uma suíte completa de testes de desempenho localizada no diretório `benchmarks/`:

```text
benchmarks/
├── run.sh                  # Script mestre de execução
├── fallback_bench.py       # Micro-benchmarker em Python (precisão de nanossegundos via time.perf_counter_ns)
├── plot.py                 # Renderizador de gráficos em ASCII e imagens PNG (matplotlib)
└── README.md               # Instruções detalhadas da suíte
```

### Como Executar os Benchmarks

Basta rodar o script a partir da raiz do projeto:

```bash
cd benchmarks
./run.sh
```

O script automaticamente:
1. Compila o `pkg` em modo otimizado (`cargo build --release`).
2. Constrói o pacote de teste sintético (`examples/hello_world/`).
3. Detecta se a ferramenta `hyperfine` está instalada no sistema:
   - Se disponível, executa medições estatísticas avançadas com aquecimento de cache (`warmup`) e comandos de preparação (`prepare`).
   - Se ausente, aciona automaticamente o motor interno `fallback_bench.py` com isolamento de ambiente (`--data-dir`).
4. Salva os resultados brutos em `benchmarks/results/` e gera gráficos comparativos via `plot.py`.

---

## 📊 Medições e Compartilhamento

The release gate categories have a dedicated reproducible driver:

```bash
./benchmarks/run-gates.sh --runs 10 --warmup 2
```

It records repository parsing, dependency solving, payload extraction,
activation/publication and transaction recovery in
`benchmarks/results/gates/`. Each category uses a fresh process; mutating
categories use fresh temporary roots. The checked-in baseline documents the
host, toolchain and exact fixture used for the measured values.

Para gerar gráficos visuais formatados em PNG após a execução dos testes:

```bash
python3 benchmarks/plot.py benchmarks/results/benchmark_*.json --png benchmarks/results/chart.png
```

Isso gera visualizações claras do tempo de execução de:
- `pkg info` (leitura de metadados e integridade)
- `pkg install --dry-run` (planejamento sem mutação de disco)
- `pkg install` (descompactação, transação, store e ativação)
- `pkg remove` (desativação e limpeza atômica)
