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

### Milestone 3: Expansão Cross-Distro e Resolvedor de Dependências (Concluído)
- [x] **Adaptadores de Artefatos Multi-Ecossistema:**
  - Suporte completo e sem dependências externas a pacotes **RPM** (`.rpm`): parser de Lead, Header de assinaturas e cabeçalho de tags; descompressão de payloads (gzip, zstd, xz) e extração de arquivos CPIO sob limites rígidos de segurança (`ExtractionLimits` - INV-004).
  - Suporte completo a pacotes **Arch Linux / ALPM** (`.pkg.tar.zst`): descompressão zstd, parsing e normalização de metadados do `.PKGINFO` e inventariação segura de scripts `.INSTALL`.
- [x] **Política Estrita de Segurança e Default-Deny (INV-003 & ADR-011):**
  - Scriptlets de mantenedor RPM (`%pre`, `%post`, `%preun`, `%postun`) e ALPM (`pre_install`, `post_install`, etc.) são inventariados com segurança como metadados, mas **nunca executados** no host.
- [x] **IR Normalizada de Restrições (ADR-009):**
  - Modelagem unificada de dependências via `Constraint` (`AllOf`, `AnyOf`, `Package`, `Capability`, `Conflict`).
  - Representação precisa de operadores de versão (`=`, `!=`, `<`, `<=`, `>`, `>=`).
- [x] **Preservação de Semântica de Versões (INV-008):**
  - Proibição absoluta de coerção para SemVer padrão.
  - Comparadores nativos independentes: `DebianVersion` (com tildes `~` e epochs), `RpmVersion` (implementação de algoritmo idêntico ao `rpmvercmp` com carets `^` e tildes) e `AlpmVersion` (`alpm_vercmp`).
- [x] **Resolvedor de Dependências Baseado em Capacidades e Evidências (ADR-016):**
  - Rejeição estrita de falsa equivalência nominal entre distribuições (INV-007: pacotes com mesmo nome em distros diferentes não são equivalentes sem prova de capacidade).
  - Verificação de evidências binárias de bibliotecas dinâmicas ELF (`DT_NEEDED` / SONAMEs - INV-009) contra os recursos fornecidos ou presentes no hospedeiro (`HostEvidence`).
  - Geração de cadeias explicativas legíveis por humanos (`ExplanationChain` - INV-020) detalhando conflitos, dependências ausentes e inconsistências de ABI.
- [x] **Gates de Validação Aprovados:**
  - Gate M3 (Resolver, Multi-Format & Evidence Compatibility) 100% aprovado (`tests/gate_m3_resolver.rs`).

---

## 🔭 Próximos Passos (Roadmap Futuro)

| Marco | Foco Principal |
|---|---|
| **Expansão de Repositórios Remotos (RPM, Arch & AUR)** | Sincronização online de metadados RPM-MD (`repomd.xml` / `primary.xml.gz`), sincronização ALPM (`core.db` / `extra.db`) e integração com Arch User Repository (AUR) via RPC v5, pacotes `-bin` e builds herméticos. |
| **Milestone 4 — Integração Desktop** | Criação tipada e reversível de arquivos `.desktop`, ícones e associações MIME no espaço do usuário sem scripts de mantenedor. |
| **Milestone 5 — Hardening v1.0** | Gerações de perfis, rollback atômico de versões instaladas, coletor de lixo (Garbage Collector da Store) e ferramenta de diagnóstico `pkg doctor`. |
