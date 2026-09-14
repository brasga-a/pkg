# Referência da CLI (Manual de Comandos)

O utilitário de linha de comando `pkg` oferece uma interface moderna, previsível e rica para gerenciamento de pacotes locais e remotos.

---

## 🧭 Opções Globais

As seguintes opções podem ser passadas para qualquer comando:

| Flag | Argumento | Padrão | Descrição |
|---|---|---|---|
| `-v`, `--verbose` | N/A | `false` | Habilita logs detalhados de diagnóstico e rastreamento interno (via `tracing`). |
| `--data-dir` | `<PATH>` | `~/.local/share/pkg` | Define um diretório base personalizado para o estado, store e perfis (útil para testes isolados). |
| `--profile` | `<NOME>` | `default` | Especifica o perfil de ativação no qual os binários serão instalados ou removidos. |
| `-h`, `--help` | N/A | N/A | Exibe ajuda e opções do comando. |
| `-V`, `--version` | N/A | N/A | Exibe a versão do `pkg`. |

---

## 📦 Comandos Principais

### 1. `pkg install`
Instala um pacote no sistema. O comando detecta automaticamente se o alvo é um arquivo `.deb` local existente ou o nome de um pacote remoto presente no catálogo.

```bash
# Instalar a partir de um arquivo local .deb
pkg install ./caminho/para/meu-pacote_1.0.0_amd64.deb

# Instalar buscando no catálogo remoto
pkg install ripgrep

# Simular a instalação sem realizar nenhuma alteração no disco ou banco de dados
pkg install ripgrep --dry-run
pkg install ./meu-pacote.deb --dry-run
```

**Comportamento:**
- **Local:** Valida a integridade do arquivo, detecta arquitetura, simula ativação de binários e promove para a store.
- **Remoto:** Busca o pacote no catálogo sincronizado do SQLite, faz o download via HTTP assíncrono para o cache (`cache/artifacts/sha256/`), valida o hash SHA-256 e executa a instalação local rootless.
- **Conflito de Binário:** Se outro pacote instalado já expõe um executável com o mesmo nome (ex: `bin/hello`), a instalação falha com erro explícito (`ActivationConflict`).

---

### 2. `pkg remove`
Desinstala um pacote previamente instalado no perfil ativo.

```bash
# Remover um pacote
pkg remove hello-world

# Simular a remoção exibindo os symlinks que seriam deletados
pkg remove hello-world --dry-run
```

**Comportamento:**
- Remove atomicamente os symlinks do perfil em `profiles/<perfil>/bin/`.
- Remove a pasta do pacote na `store/` se não houver referências pendentes.
- Atualiza o registro no SQLite de forma idempotente e segura.

---

### 3. `pkg list`
Lista todos os pacotes instalados no perfil ativo.

```bash
pkg list
```

**Exemplo de saída:**
```text
NAME                 VERSION         STATUS     FORMAT   ACTIVE   STORE_ID
hello-world          1.0.0           installed  deb      yes      deb-hello-world-1.0.0-e3b0c442...
discord              0.0.98          installed  deb      yes      deb-discord-0.0.98-9f82ab11...
```

---

### 4. `pkg info`
Exibe metadados detalhados sobre um pacote instalado ou inspeciona um arquivo `.deb` local antes de instalar.

```bash
# Inspecionar pacote já instalado
pkg info hello-world

# Inspecionar arquivo .deb local sem instalar
pkg info ./examples/hello_world/hello-world_1.0.0_amd64.deb
```

**Informações exibidas:**
- Identidade: Nome, versão, arquitetura, formato e descrição.
- Proveniência: Digest SHA-256, tamanho do artefato e tamanho instalado.
- Recursos fornecidos (`Provides`) e dependências.
- Binários ativados e symlinks gerados.
- Inventário de Scripts de Mantenedor identificados (com indicação da política de segurança `default-deny`).

---

### 5. `pkg update`
Sincroniza o catálogo local com os repositórios remotos configurados.

```bash
pkg update
```

**Comportamento:**
- Lê a lista de repositórios declarada em `repositories.toml`. Se o arquivo não existir, cria um arquivo padrão contendo os repositórios do **Ubuntu Noble** e **Debian Bookworm**.
- Baixa o arquivo `InRelease` de cada repositório e **valida criptograficamente sua assinatura GPG**.
- Baixa e descompacta os índices de pacotes (`Packages.xz` ou `Packages.gz`).
- Executa um commit atômico no banco de dados SQLite local, substituindo o catálogo antigo pelo novo snapshot.
- Se o download ou validação falhar, o catálogo anterior permanece ativo sem corrupção.

---

### 6. `pkg search`
Realiza buscas instantâneas por pacotes disponíveis no catálogo local atualizado.

```bash
pkg search ripgrep
pkg search discord
```

**Comportamento:**
- Não faz requisições de rede. Consulta diretamente os índices indexados no SQLite (`remote_packages`), retornando resultados em sub-milissegundos.

---

## 🌐 Gerenciamento de Repositórios (`pkg repo`)

Permite inspecionar e adicionar novas fontes de pacotes sem a necessidade de editar arquivos de texto manualmente.

### `pkg repo list`
Lista todos os repositórios configurados no sistema:

```bash
pkg repo list
```

**Exemplo de saída:**
```text
ID                        URL                                 DISTRIBUTION    COMPONENTS
ubuntu-noble              http://archive.ubuntu.com/ubuntu    noble           main, universe, restricted, multiverse
debian-bookworm           http://deb.debian.org/debian        bookworm        main, contrib, non-free
```

### `pkg repo add`
Adiciona um novo repositório ao arquivo `repositories.toml`:

```bash
# Sintaxe: pkg repo add <ID> <URL> <DISTRIBUIÇÃO> [COMPONENTES...]
pkg repo add debian-sid http://deb.debian.org/debian sid main contrib non-free
```
Após adicionar, basta rodar `pkg update` para indexar os novos pacotes.
