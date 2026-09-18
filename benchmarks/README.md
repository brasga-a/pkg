# pkg Benchmarking Suite

Esta suíte mede o `pkg` em estado controlado e, opcionalmente, compara o caminho de instalação end-to-end com `dpkg -i`.

O objetivo principal é produzir números **reproduzíveis e auditáveis**, não apenas o menor tempo possível.

## Estrutura

```text
benchmarks/
├── README.md             # Este guia completo de metodologia e execução
├── run.sh                # Script principal de execução dos testes
├── run-gates.sh          # Baseline dos gates de parsing/solver/extraction/recovery
├── cli_commands.sh       # Suite de comandos CLI (dry-run, list, info)
├── fallback_bench.py     # Executor de benchmark nativo em Python
├── merge_results.py      # Agregador de resultados e relatórios percentis
├── resource_bench.py     # Coletor de métricas de CPU, RSS e I/O
├── plot.py               # Gerador de gráficos (gráfico ASCII e PNG)
└── results/              # Pasta onde os relatórios (Markdown, JSON, PNG) são salvos
    └── <timestamp>-<package>-<suite>-<warm|cold>/
        ├── environment.txt
        ├── results.json
        ├── results.md
        ├── resources.json
        ├── resources.md
        ├── benchmark_chart.png
        └── raw/
```

`results/` permanece ignorado pelo Git porque os resultados devem ser associados ao hardware/ambiente que os produziu.

## Pré-requisitos

Compile primeiro em release:

```bash
cargo build --release
```

`hyperfine` é recomendado:

```bash
cargo install hyperfine
```

Sem ele, a suíte usa `fallback_bench.py`. O fallback aborta se um comando ou preparação falhar; falhas não são contabilizadas como amostras válidas.

Para as métricas de CPU/memória/I/O é usado GNU `/usr/bin/time -v`, quando disponível.

## Suite end-to-end / operações principais

Benchmark local do `pkg`:

```bash
./benchmarks/run.sh
./benchmarks/run.sh ~/Downloads/discord.deb
```

Comparação com `dpkg`:

```bash
./benchmarks/run.sh ~/Downloads/discord.deb --compare-dpkg
```

Mais amostras:

```bash
./benchmarks/run.sh ~/Downloads/discord.deb \
  --compare-dpkg \
  --runs 30 \
  --warmup 5
```

Fixar os processos medidos em um CPU lógico:

```bash
./benchmarks/run.sh ~/Downloads/discord.deb \
  --compare-dpkg \
  --cpu 2
```

Cold-cache:

```bash
./benchmarks/run.sh ~/Downloads/discord.deb \
  --compare-dpkg \
  --cache-mode cold \
  --runs 10
```

### Opções do `run.sh`

- `--compare-dpkg`: mede `pkg install` e `dpkg -i` em instalações limpas independentes;
- `--runs N`: número de execuções cronometradas, padrão 15;
- `--warmup N`: rodadas descartadas antes das medições, padrão 3;
- `--cache-mode warm|cold`: política de page cache, padrão `warm`;
- `--cpu LIST`: aplica `taskset -c` apenas ao comando medido;
- `--resource-runs N`: amostras adicionais para CPU/RSS/I/O, padrão 5;
- `--no-resources`: desativa a passagem com `/usr/bin/time -v`;
- `--output-dir DIR`: altera a raiz dos resultados.

### Baseline dos gates de release (`run-gates.sh`)

Para medir as categorias exigidas pelo Gate M5 com fixtures locais e raízes temporárias isoladas:

```bash
./benchmarks/run-gates.sh --runs 10 --warmup 2
```

O driver mede parsing de índice Debian, resolução, extração, publicação da ativação e recuperação de uma transação incompleta. Os resultados ficam em `benchmarks/results/gates/` e incluem JSON bruto e tabela Markdown.

## Suite de comandos do CLI

`cli_commands.sh` cobre caminhos do CLI que exigem estados diferentes e não devem ser misturados com o benchmark end-to-end de instalação.

Comandos medidos:

```text
pkg install <artifact> --dry-run
pkg remove <name> --dry-run
pkg list                  # estado vazio
pkg list                  # pacote instalado
pkg info <name>           # pacote instalado
```

Execução padrão:

```bash
./benchmarks/cli_commands.sh
```

Com um artifact real e mais amostras:

```bash
./benchmarks/cli_commands.sh ~/Downloads/discord.deb \
  --runs 50 \
  --warmup 10 \
  --cpu 2
```

Cold-cache:

```bash
./benchmarks/cli_commands.sh \
  --cache-mode cold \
  --runs 20
```

Outro profile:

```bash
./benchmarks/cli_commands.sh --profile perf-test
```

### Metodologia da suite de CLI

A suite constrói duas baselines imutáveis antes de iniciar qualquer medição:

```text
initialized empty baseline
          |
          +------------------------------+
          |                              |
          v                              v
     empty state                 installed state
                                       |
                               fixture installed once
```

Antes de cada amostra, a baseline adequada é copiada para um `--data-dir` descartável:

- `install --dry-run` e `list empty` usam a baseline vazia;
- `remove --dry-run`, `list installed` e `info <name>` usam a baseline já instalada;
- a instalação usada para construir a baseline instalada ocorre **fora** da janela cronometrada;
- o profile é fixado explicitamente para evitar que estado de outros profiles contamine o teste.

Os cinco comandos têm semânticas diferentes. Por isso `results.md` **não calcula speedup relativo entre eles**. O uso correto é acompanhar regressões de cada comando contra seu próprio histórico.

### Opções do `cli_commands.sh`

- `--runs N`: número de execuções cronometradas, padrão 30;
- `--warmup N`: rodadas descartadas, padrão 5;
- `--cache-mode warm|cold`: política de page cache;
- `--cpu LIST`: CPU affinity do comando medido;
- `--profile NAME`: profile usado pela baseline instalada;
- `--resource-runs N`: amostras adicionais de recursos;
- `--no-resources`: desativa coleta via GNU time;
- `--output-dir DIR`: altera a raiz dos resultados.

## Estado reproduzível

### pkg

As suítes **não usam `~/.local/share/pkg`**.

Elas criam `--data-dir` temporários e restauram baselines conhecidas antes das execuções. Isso evita que histórico de transações, SQLite, store ou profiles de runs anteriores mudem o custo da execução seguinte.

No benchmark principal, a baseline é vazia:

```text
empty initialized baseline
        |
        | copy before every run
        v
throwaway pkg data-dir
        |
     timed install
```

Na suite de CLI, a baseline é escolhida de acordo com a precondição semântica do comando.

A preparação sempre fica fora da janela medida.

### dpkg

No modo `--compare-dpkg`, a suíte:

1. autentica `sudo` antes do benchmark;
2. recusa executar se o pacote já estiver registrado no banco local do `dpkg`;
3. executa `dpkg --purge` fora da janela medida antes de cada run;
4. verifica que o pacote realmente saiu do banco;
5. usa `sudo -n` no comando medido para nunca contabilizar prompt de senha;
6. faz uma limpeza best-effort ao sair.

**Use uma VM descartável para comparações com `dpkg`.** Maintainer scripts podem realizar efeitos de sistema que um `dpkg --purge` posterior não necessariamente desfaz completamente.

## Warm vs cold

Estado do package manager e page cache são variáveis diferentes.

### `warm` — padrão

Cada run começa de um estado lógico limpo, mas o Linux pode manter páginas do artifact/binários no page cache. É o modo de menor variância e representa bem operações repetidas em uma workstation.

### `cold`

Antes de cada comando medido a suíte executa `sync` e limpa o page cache do Linux via `/proc/sys/vm/drop_caches`.

Isso:

- exige `sudo`;
- afeta a máquina inteira;
- não deve ser usado enquanto outros workloads importantes estão rodando;
- deve ser executado preferencialmente em VM/host de benchmark.

Nunca misture resultados warm e cold em uma mesma média.

## `pkg` vs `dpkg`: o que o número significa

Esta comparação é válida como **latência do caminho de instalação end-to-end**, mas os dois comandos não realizam trabalho semanticamente idêntico.

`pkg install` atualmente:

- é rootless;
- materializa em store isolado;
- atualiza seu SQLite e activation profile;
- default-denies maintainer scripts.

`dpkg -i`:

- escreve em paths nativos do sistema;
- atualiza o banco do dpkg;
- executa semântica Debian/maintainer scripts aplicável;
- opera com privilégios de root.

Portanto, um resultado como `pkg 5.6x faster than dpkg` **não significa** que o parser/engine Rust é 5.6x mais rápido executando o mesmo trabalho. A formulação correta é algo como:

> Neste artifact, hardware e cache mode, o caminho end-to-end atualmente implementado por pkg teve X vezes menor wall-clock latency que `dpkg -i`.

Para comparar a eficiência pura de parsers, extraction, hashing, SQLite ou ELF inspection, use micro/component benchmarks com trabalho equivalente.

## Métricas

`results.md` inclui:

- média ± desvio padrão;
- mediana;
- p95;
- mínimo/máximo;
- ratio relativo apenas quando as operações são marcadas como comparáveis.

`resources.md` inclui médias de:

- wall clock;
- user CPU;
- system CPU;
- peak RSS;
- filesystem input/output operations;
- major page faults.

`resources.json` preserva também os samples e outros contadores do GNU time, incluindo minor faults e context switches.

> `File system inputs/outputs` do GNU time são contadores de operações, não bytes.

## Evidência do ambiente

Cada run gera `environment.txt` com:

- commit Git;
- artifact + SHA-256 + tamanho;
- distro/kernel;
- CPU/model/count/governor quando disponível;
- versões de Rust/Cargo/pkg/hyperfine/dpkg;
- warm/cold;
- runs/warmups;
- CPU affinity;
- profile, quando aplicável;
- linha de comando usada.

Ao publicar resultados, publique também esse arquivo.

## Corpus recomendado

Não tire conclusões gerais a partir apenas do Discord. Use pelo menos classes diferentes:

1. fixture minúscula/self-contained;
2. CLI normal dinamicamente linkada;
3. aplicação/CLI com metadata e dependências mais complexas;
4. desktop app grande, como Discord.

Idealmente, rode o mesmo corpus em mais de uma máquina e distro.

## Redução de ruído

Para resultados destinados a comparação pública:

- use uma máquina/VM ociosa;
- conecte o notebook à energia;
- registre o CPU governor;
- opcionalmente use `--cpu` para reduzir migração entre cores;
- rode 20–50 amostras para operações curtas;
- mantenha artifact e filesystem iguais;
- reporte valores absolutos junto do ratio;
- não remova outliers manualmente sem uma regra estatística documentada.

A política arquitetural completa está em [`context/research/benchmarks.md`](../context/research/benchmarks.md).
