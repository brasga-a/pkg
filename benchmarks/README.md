# pkg Benchmarking Suite

Este diretório contém uma suíte completa de benchmarking para o `pkg`, permitindo medir latência de instalação, remoção e inspeção de pacotes `.deb`, além de comparar o desempenho de forma justa e reproduzível contra ferramentas tradicionais como `dpkg` e `apt`.

---

## Estrutura dos Arquivos

```
benchmarks/
├── README.md             # Este guia completo de metodologia e execução
├── run.sh                # Script principal de execução dos testes
├── fallback_bench.py     # Executor de benchmark nativo em Python (quando hyperfine não estiver instalado)
├── plot.py               # Gerador de gráficos (gráfico ASCII no terminal e PNG com matplotlib)
└── results/              # Pasta onde os relatórios (Markdown, JSON, PNG) são salvos
```

---

## Pré-requisitos Recomendados

Para a medição mais precisa possível, recomendamos ter o [`hyperfine`](https://github.com/sharkdp/hyperfine) instalado:

```bash
cargo install hyperfine
```

> **Nota:** Se você não tiver o `hyperfine` instalado, o `run.sh` utilizará automaticamente o `fallback_bench.py`, medindo os tempos com precisão de nanossegundos (`time.perf_counter_ns`) em Python 3 puro.

Certifique-se também de compilar o `pkg` em modo otimizado:
```bash
cargo build --release
```

---

## Como Executar

### 1. Benchmark Padrão (Pacote `hello-world`)
Executa medições de `pkg info`, `pkg install` e `pkg remove` no pacote de exemplo:

```bash
./benchmarks/run.sh
```

### 2. Benchmark com um Pacote Real (ex: Discord, VS Code, Ripgrep)
Basta passar o caminho do `.deb` como argumento:

```bash
./benchmarks/run.sh /caminho/para/discord.deb
```

### 3. Comparativo com o `dpkg` (`--compare-dpkg`)
Para medir o `pkg` lado a lado com o `dpkg -i` tradicional do sistema operacional:

```bash
# Aqueça a senha do sudo primeiro para que ela não seja contada no tempo
sudo true

# Execute o comparativo
./benchmarks/run.sh /caminho/para/discord.deb --compare-dpkg
```

### 4. Opções Adicionais
- `--runs <N>`: Número de repetições por comando (padrão: 15).
- `--warmup <N>`: Número de rodadas de aquecimento descartadas (padrão: 3).
- `--output-dir <DIR>`: Diretório para salvar os relatórios gerados (padrão: `benchmarks/results`).

---

## Metodologia Científica e Justa

Para garantir que os resultados sejam confiáveis e aceitos por comunidades como **r/rust**, **r/linux** e **Hacker News**:

1. **Warmup (Aquecimento de Cache):**
   - São executadas rodadas preliminares para que o cache de I/O do sistema de arquivos e o carregamento do binário não enviesem a primeira execução.
2. **Ambiente Limpo a Cada Rodada (`--prepare`):**
   - Antes de cada medição de instalação, o pacote anterior é limpo para que o teste sempre meça uma instalação limpa e real.
3. **Isolamento de Processo:**
   - O `pkg` roda em modo rootless (`~/.local/share/pkg`), medindo tanto o tempo de descompressão, gravação em disco, transação no SQLite quanto a ativação de symlinks.
4. **Tratamento de Outliers:**
   - O `hyperfine` calcula média aritmética ($\mu$) e desvio padrão ($\sigma$), alertando caso algum processo em segundo plano interfira nas medições.

---

## Gerando Gráficos para Publicação

Após rodar o benchmark, os resultados são salvos em `benchmarks/results/results.json`. Para gerar gráficos visuais:

```bash
python3 benchmarks/plot.py
```

- Exibe um **gráfico de barras ASCII** diretamente no terminal (ótimo para copiar para markdown).
- Se a biblioteca `matplotlib` estiver instalada (`pip install matplotlib`), gera uma imagem `results.png` em alta definição e tema escuro, pronta para postar no Twitter/X, Reddit ou LinkedIn.

---

## Dicas para Postar nas Redes Sociais

Ao publicar os resultados:
1. **Compartilhe o Contexto Técnico:**
   - Enfatize que o `pkg` é **rootless** (não precisa de `sudo`).
   - Explique que o ganho vem do streaming puro em memória em Rust (`flate2`/`xz2`), banco SQLite WAL transacional e ativação de symlinks sem scripts `postinst` pesados.
2. **Adicione as Tabelas do `results.md`:**
   - A tabela Markdown gerada em `benchmarks/results/results.md` pode ser colada diretamente em posts do Reddit ou GitHub Issues/Discussions.
3. **Inclua o link do repositório:**
   - Permita que outras pessoas reproduzam os mesmos números facilmente rodando `./benchmarks/run.sh`.
