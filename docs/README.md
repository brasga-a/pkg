# Documentação do `pkg`

Bem-vindo à documentação técnica e de uso do **`pkg`**, o gerenciador de pacotes universal e cross-distribution para Linux desenvolvido em **Rust (edição 2024)**.

O `pkg` foi projetado para resolver a fragmentação de distribuição de software no ecossistema Linux através de uma arquitetura estritamente **rootless**, com **isolamento de arquivos** (isolated store), **transações atômicas com recuperação automática**, e compatibilidade com múltiplos ecossistemas (iniciando com `.deb` e repositórios Debian/Ubuntu).

---

## 📚 Índice da Documentação

A documentação está dividida nos seguintes módulos detalhados:

1. [**Arquitetura do Sistema**](architecture.md)
   - Princípios de design, Clean Architecture e divisão em crates (`pkg-core`, `pkg-cli`).
   - Estrutura de diretórios em espaço de usuário (`~/.local/share/pkg/`).
   - Ciclo de vida transacional, modelo de recuperação de falhas (crash resilience) e invariantes de segurança.

2. [**Referência da CLI (Manual de Comandos)**](cli-reference.md)
   - Guia prático de todos os comandos implementados: `install`, `remove`, `list`, `info`, `update`, `search`, `repo`.
   - Modos de simulação (`--dry-run`), gestão de perfis (`--profile`) e flags globais.

3. [**Catálogo Remoto e Confiança Criptográfica (M2)**](repositories-and-trust.md)
   - Arquitetura de estado duplo (TOML para humanos + SQLite para performance sub-milissegundo).
   - Validação de assinaturas GPG em arquivos `InRelease`.
   - Cache de artefatos endereçado por digest (SHA-256) e cliente de download seguro (`BoundedDownloader`).

4. [**Histórico de Marcos e Roadmap**](milestones.md)
   - Detalhamento do que foi entregue nos **Milestones 0, 1 e 2**.
   - Próximos passos (M3: RPM/Arch, M4: Desktop Integration, M5: Gerações/Rollback/GC).

5. [**Benchmarks e Performance**](benchmarks.md)
   - Metodologia de teste, suíte de benchmarks automatizada com `hyperfine` e fallback Python.
   - Análise de por que a instalação com `pkg` é quase instantânea em comparação ao `apt`.

---

## ⚡ Guia Rápido (Quickstart)

### 1. Compilação
Certifique-se de estar com a versão mais recente do Rust instalada:
```bash
cargo build --release
```
O executável final estará disponível em `target/release/pkg`.

### 2. Adicionando ao `$PATH` (Opcional)
Para usar o comando globalmente e permitir que os binários ativados sejam executados no terminal:

```bash
# Adiciona o diretório do binário do pkg
export PATH="$HOME/projects/pkg/target/release:$PATH"

# Adiciona o profile padrão de execução de pacotes do pkg
export PATH="$HOME/.local/share/pkg/profiles/default/bin:$PATH"
```
*(Dica: adicione as linhas acima ao seu `~/.bashrc` ou `~/.zshrc`).*

### 3. Primeiros Comandos

```bash
# 1. Listar repositórios padrão (Debian / Ubuntu)
pkg repo list

# 2. Atualizar o catálogo remoto e validar assinaturas
pkg update

# 3. Buscar um pacote disponível
pkg search ripgrep

# 4. Instalar um pacote local .deb com segurança rootless
pkg install ./examples/hello_world/hello-world_1.0.0_amd64.deb

# 5. Testar execução do comando ativado
hello-world

# 6. Inspecionar detalhes do pacote instalado
pkg info hello-world

# 7. Remover o pacote e desativar seus symlinks
pkg remove hello-world
```
