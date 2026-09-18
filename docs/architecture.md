# Arquitetura do Sistema

O `pkg` é construído sob uma arquitetura limpa (Clean Architecture / Hexagonal) em Rust 2024, desacoplando o núcleo de domínio de adaptadores de formatos de pacotes, armazenamento, banco de dados e interface de usuário (CLI).

---

## 🏗️ Visão Geral dos Componentes

O projeto é organizado como um workspace Cargo contendo duas crates principais:

```text
├── Cargo.toml                  # Workspace raiz com dependências unificadas e lints rígidos
├── crates/
│   ├── pkg-core/               # Núcleo de lógica de domínio, segurança e transações
│   └── pkg-cli/                # Interface de linha de comando (Clap + Tokio)
├── examples/                   # Pacotes de exemplo (.deb) e scripts de build
├── benchmarks/                 # Suíte automatizada de testes de performance
└── docs/                       # Documentação técnica completa
```

### 1. `crates/pkg-core` (A Camada de Domínio e Mecanismo)
Responsável por toda a lógica de negócios, garantindo que nenhuma regra de segurança ou integridade seja violada. Não possui dependências diretas de CLI (`clap`).

- **`domain/`**: Entidades fundamentais puras (`Package`, `InstalledPackage`, `RemotePackage`, `InstallPlan`, `RemovePlan`, etc.).
- **`format/`**: Trait `ArtifactAdapter` e implementações (atualmente `deb.rs`), realizando a leitura segura de arquivos de pacote em Rust puro (`ar`, `tar`, `flate2`, `xz2`, `zstd`).
- **`store/`**: Mapeamento do diretório do usuário (`StoreLayout`), gestão de diretórios isolados por hash e criação atômica de symlinks.
- **`state/`**: Gerenciamento do banco de dados local SQLite via `rusqlite`, garantindo consistência com `PRAGMA journal_mode = WAL`.
- **`transaction/`**: Execução de transações em estágios duráveis e recuperação de falhas/crashes (`recovery.rs`).
- **`planner/`**: Planejamento de instalação e remoção sem efeitos colaterais (`side-effect-free`), validando arquitetura, dependências e conflitos de comandos antes de tocar no disco.
- **`transport/`**: Cliente de rede assíncrono (`BoundedDownloader`) baseado em `tokio` e `reqwest` com limites estritos de taxa e tamanho.
- **`repository/`**: Configuração (`config.rs`), adaptadores de repositórios remotos (`deb.rs`) e validação de assinaturas criptográficas PGP/GPG.
- **`lock/`**: Process lock (`flock`) de escritor único (`single-writer`) para prevenir concorrência indesejada entre instâncias do `pkg`.
- **`host/`**: Detecção de fatos do sistema operacional hospedeiro (arquitetura, bibliotecas ELF).
- **`engine.rs`**: Ponto de entrada de alto nível que orquestra todos os subsistemas.

### 2. `crates/pkg-cli` (A Borda da Aplicação)
- Construído com `clap` (derivação declarativa) e `anyhow` para tratamento amigável de erros.
- Ponto de entrada com `#[tokio::main]`, permitindo orquestração assíncrona de rede e operações de terminal concorrentes.

---

## 🔒 Princípios e Invariantes Fundamentais

O desenvolvimento do `pkg` é estritamente regido pelas seguintes invariantes arquiteturais:

| ID | Princípio | Descrição |
|---|---|---|
| **INV-001** | **Sem mutação nativa** | O `pkg` nunca altera `/var/lib/dpkg`, banco de dados RPM ou libalpm do sistema hospedeiro. |
| **INV-002** | **Default Rootless** | Todo o estado, downloads, binários e metadados residem dentro do `$HOME` do usuário. `sudo` é desnecessário. |
| **INV-003** | **Default-Deny para Scripts** | Scripts de mantenedor (`preinst`, `postinst`, `prerm`, `postrm`) contidos em pacotes estrangeiros **nunca** são executados no host. |
| **INV-004** | **Store Isolada** | Os arquivos do pacote são descompactados em um diretório próprio e imutável, endereçado pelo hash do conteúdo. |
| **INV-005** | **Identidade Verificada** | Artefatos e metadados devem ter sua autenticidade (GPG) e integridade (SHA-256) validadas antes da promoção. |
| **INV-010** | **Planejamento Sem Efeitos** | A geração de `InstallPlan` e o modo `--dry-run` não criam arquivos, não alocam transações e não tocam no banco. |
| **INV-012** | **Single Writer Lock** | Apenas um processo `pkg` por usuário pode executar alterações no disco por vez via `pkg.lock`. |
| **INV-013** | **Recuperabilidade** | Se o processo for interrompido abruptamente (`SIGKILL`, falta de energia), a próxima execução desfaz o estado parcial automaticamente. |

---

## 📁 Estrutura de Diretórios (`StoreLayout`)

Por padrão, o `pkg` utiliza o diretório `~/.local/share/pkg/` (respeitando a especificação XDG):

```text
~/.local/share/pkg/
├── pkg.lock                    # Trava de processo exclusiva (advisory flock)
├── repositories.toml           # Arquivo de configuração de repositórios (editável pelo usuário)
│
├── store/                      # Store isolada e imutável de pacotes descompactados
│   └── <store-id>/             # Ex: deb-hello-world-1.0.0-sha256-...
│       ├── usr/
│       │   └── bin/hello-world
│       └── ...
│
├── staging/                    # Área transitória para descompactação e verificação
│   └── <tx-id>/                # Excluído atomicamente após sucesso ou recuperação
│
├── profiles/                   # Perfis de ambiente
│   └── default/
│       └── bin/                # Diretório que o usuário adiciona ao seu $PATH
│           └── hello-world -> ../../store/<store-id>/usr/bin/hello-world
│
├── cache/                      # Cache persistente
│   └── artifacts/
│       └── sha256/             # Arquivos .deb baixados, nomeados pelo seu digest SHA-256
│           └── e3b0c44298fc...
│
└── state/
    ├── pkg.db                  # Banco de dados local SQLite
    ├── pkg.db-wal              # Write-Ahead Log do SQLite
    └── pkg.db-shm              # Memória compartilhada do SQLite
```

---

## 🔄 Ciclo de Vida Transacional

Toda instalação de pacote segue uma máquina de estados estrita:

```mermaid
stateDiagram-v2
    [*] --> Planned: Planner::plan_install()
    Planned --> Staging: Cria Staging Dir + Registra TX no SQLite
    Staging --> Verifying: Descompacta Payload com Validação de Caminhos
    Verifying --> Promoting: std::fs::rename() atômico para a Store
    Promoting --> Activating: Cria symlinks atômicos em profiles/default/bin
    Activating --> Committed: Registra pacote e arquivos no SQLite
    Committed --> [*]

    Staging --> FailedClean: Erro durante extração / SIGINT
    Verifying --> FailedClean: Falha na validação de integridade
    Promoting --> FailedClean: Falha ao promover
    Activating --> FailedClean: Conflito de binário não resolvido
    FailedClean --> [*]: Limpa staging dir e remove órfãos
```

### Mecanismo de Recuperação de Falhas (`Recovery`)
Se a máquina for desligada ou o comando finalizado à força durante o estágio `Staging`:
1. Na próxima inicialização de `Engine::open()`, o método `Recovery::reconcile()` é acionado.
2. O SQLite é consultado em busca de transações que não atingiram a fase `Committed`.
3. O diretório transitório `staging/<tx-id>` correspondente é removido do disco.
4. Se uma pasta na `store/` foi criada mas não registrada na tabela de pacotes ativos, ela é tratada como órfã e excluída.
5. A transação é atualizada para `FailedClean`, impedindo qualquer corrupção de estado.
