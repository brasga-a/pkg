# Histórico de Marcos e Roadmap

Este documento resume a evolução do desenvolvimento do `pkg`, consolidando o que já foi implementado e os próximos passos planejados.

---

## 🏁 Marcos Concluídos

### Milestone 0: Fundação do Projeto (Concluído)
- [x] Configuração do Workspace Cargo em **Rust 2024** (edição estável).
- [x] Definição de lints estritos (`clippy`, `rust_2018_idioms`, `unused_must_use`).
- [x] Separação de arquitetura: `crates/pkg-core` (domínio) e `crates/pkg-cli` (linha de comando).
- [x] Definição e catálogo formal de ADRs (Architecture Decision Records) e Invariantes do sistema.
- [x] Hierarquia de erros tipados (`thiserror`) em `pkg-core` e relatórios contextuais (`anyhow`) em `pkg-cli`.

---

### Milestone 1: Kernel de Pacotes Locais `.deb` (Concluído)
- [x] **Parser Seguro em Rust Puro:**
  - Descompactação de contêineres `ar` e arquivos `tar` (`tar.gz`, `tar.xz`, `tar.zst`) sem invocar binários externos (`dpkg-deb`, `tar`).
  - Proteção contra *path traversal* (`../`), referências absolutas, links simbólicos que escapam da raiz e zip-bombs (`ExtractionLimits`).
  - Tratamento resiliente para diretórios raiz (`./`).
- [x] **Store Isolada e Rootless:**
  - Layout seguro sob `~/.local/share/pkg/`.
  - Ativação de binários de `bin/` e `usr/bin/` via symlinks atômicos em `profiles/default/bin/`.
  - Detecção e bloqueio explícito de colisões de comandos (`ActivationConflict`).
- [x] **Motor Transacional e Resiliência a Falhas:**
  - Estados: `Planned` ➔ `Staging` ➔ `Verifying` ➔ `Promoting` ➔ `Activating` ➔ `Committed`.
  - Mecanismo de recuperação na inicialização (`Recovery::reconcile()`) para desfazer transações interrompidas abruptamente.
  - Banco de dados de estado SQLite configurado em modo WAL (`Write-Ahead Logging`).
  - Trava de processo de escritor único (`ProcessLock`) usando `flock` consultivo.
- [x] **Política de Segurança:**
  - Inventariação de scripts de mantenedor (`preinst`, `postinst`, etc.) com política estrita de **Default-Deny** (não execução no host).
- [x] **Comandos CLI Locais:**
  - `pkg install <arquivo.deb>`, `pkg remove <nome>`, `pkg list`, `pkg info <alvo>` e suporte a `--dry-run`.
- [x] **Gates de Validação Aprovados:**
  - Gate M1-A (Parser & Security), Gate M1-B (Store & Activation), Gate M1-C (Transactions & Crash Recovery).

---

### Milestone 2: Catálogo Remoto e Confiança (Concluído)
- [x] **Runtime Assíncrono:**
  - Adoção de `tokio` e `reqwest` para orquestração de rede concorrente.
- [x] **Transporte HTTP Seguro (`BoundedDownloader`):**
  - Streaming assíncrono direto para disco sem sobrecarregar a memória RAM.
  - Limitação rígida de tamanho máximo de download (prevenção contra exaustão de disco).
- [x] **Sincronização de Repositórios Debian:**
  - Leitura remota de índices `InRelease`, `Packages.xz` e `Packages.gz`.
  - Validação de assinaturas criptográficas OpenPGP (`pgp` crate) em mensagens cleartext.
- [x] **Arquitetura de Estado Duplo (TOML + SQLite):**
  - Configuração declarativa transparente em `~/.local/share/pkg/repositories.toml`.
  - Geração automática de repositórios padrão para **Ubuntu Noble** e **Debian Bookworm**.
  - Normalização e inserção em snapshots atômicos no SQLite para consultas sub-milissegundo.
- [x] **Cache de Artefatos Endereçado por Digest:**
  - Armazenamento em `cache/artifacts/sha256/<hash>`.
  - Validação cruzada de SHA-256 pós-download contra a assinatura do repositório.
- [x] **Novos Comandos CLI:**
  - `pkg update`: Atualização de repositórios e compilação de snapshots.
  - `pkg search <termo>`: Busca instantânea no catálogo local.
  - `pkg repo list` e `pkg repo add`: Gerenciamento declarativo via CLI.
  - `pkg install <nome>`: Resolução automática de pacotes remotos, download para o cache e instalação na store.

---

## 🔭 Próximos Passos (Roadmap Futuro)

| Marco | Foco Principal |
|---|---|
| **Milestone 3 — Expansão Cross-Distro** | Adaptadores para formatos RPM (`.rpm`) e Arch Linux (`.pkg.tar.zst`), representação intermediária de restrições (IR) e resolvedor de dependências baseado em capacidades. |
| **Milestone 4 — Integração Desktop** | Criação tipada e reversível de arquivos `.desktop`, ícones e associações MIME no espaço do usuário. |
| **Milestone 5 — Hardening v1.0** | Gerações de perfis, rollback atômico de versões instaladas, coletor de lixo (Garbage Collector da Store) e ferramenta de diagnóstico `pkg doctor`. |
