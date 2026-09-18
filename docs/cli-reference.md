# Referência da CLI (Manual de Comandos)

O utilitário de linha de comando `pkg` oferece uma interface moderna, previsível e rica para gerenciamento de pacotes locais e remotos.

---

## ⚡ Instalação Rápida

Para instalar ou atualizar o `pkg` diretamente no seu ambiente Linux de usuário:

```bash
curl -fsSL https://pkg.atlantic.sh/install | sh
```

Ou especificando opções customizadas:

```bash
curl -fsSL https://pkg.atlantic.sh/install | bash -s -- --dir ~/.local/bin --version 0.1.0-beta.1
```

---

## 🧭 Opções Globais

As seguintes opções podem ser passadas para qualquer comando:

| Flag | Argumento | Padrão | Descrição |
|---|---|---|---|
| `-v`, `--verbose` | N/A | `false` | Habilita logs detalhados de diagnóstico e rastreamento interno (via `tracing`). |
| `--data-dir` | `<PATH>` | `~/.local/share/pkg` | Define um diretório base personalizado para o estado, store e perfis (útil para testes isolados). |
| `--profile` | `<NOME>` | `default` | Especifica o perfil de ativação no qual os binários serão instalados ou removidos. |
| `--json` | N/A | `false` | Emite um único documento JSON determinístico em stdout. |
| `--non-interactive` | N/A | `false` | Desabilita prompts e falha imediatamente diante de ambiguidades. |
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
- **Remoto:** Busca o pacote no catálogo sincronizado do SQLite, faz o download via HTTP assíncrono para o cache (`cache/artifacts/sha256/`), valida o hash SHA-256 e executa a instalação local rootless. Em `--dry-run`, o artefato precisa já estar no cache; sem os bytes locais a prévia fica incompleta e o comando falha.
- **Conflito de Binário:** Outro pacote, arquivo do usuário ou link divergente no destino causa `ActivationConflict`. Uma atualização só substitui um link cujo destino corresponde à propriedade registrada no banco.
- **Validação antes da promoção:** Versões que não podem compor um caminho seguro, entradas duplicadas, links que escapam do staging e ELF inválido ou com bibliotecas obrigatórias ausentes são rejeitados. Links internos devem resolver dentro do pacote; links pendentes ou cíclicos são rejeitados nesta implementação.
- **Cache:** Entradas existentes têm tamanho e SHA-256 revalidados. Downloads usam arquivos temporários e só recebem o nome definitivo após a verificação.

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
- Remove os symlinks do perfil em `profiles/<perfil>/bin/` quando o destino ainda corresponde ao registrado. Arquivos ou links substituídos pelo usuário são preservados.
- Remove a pasta do pacote na `store/` se não houver referências pendentes.
- Atualiza o registro no SQLite de forma idempotente e segura.

---

### 3. `pkg gc`
Analisa e, quando solicitado, remove objetos do store que não têm referências conhecidas no estado do `pkg`.

```bash
# Sempre revisar os candidatos primeiro
pkg gc --dry-run

# Coletar somente objetos conhecidos e comprovadamente inalcançáveis
pkg gc
```

**Comportamento:**
- `--dry-run` não cria o estado em uma raiz vazia, não baixa artefatos e não altera store, perfil ou banco.
- O relatório considera pacotes ativos, ativações e transações incompletas como referências. Diretórios não registrados, caminhos divergentes e objetos com estado incerto são preservados.
- A coleta revalida as referências sob o lock de escrita e remove somente diretórios filhos válidos do store; databases nativos e arquivos do usuário não são tocados.
- A primeira implementação não coleta automaticamente cache, gerações retidas ou objetos potencialmente usados por processos em execução.

---

### 4. `pkg rollback`
Seleciona uma geração previamente publicada do perfil e restaura também o estado lógico registrado no SQLite.

```bash
pkg rollback                 # volta para a geração anterior
pkg rollback gen-tx-...      # seleciona uma geração específica
```

As gerações são imutáveis. O comando falha se o manifesto ou algum objeto referenciado não estiver disponível.

### 5. `pkg run`
Executa um comando pelo manifesto de runtime da geração ativa, aplicando apenas as bibliotecas selecionadas para aquele comando e removendo overrides de loader herdados do ambiente.

```bash
pkg run meu-comando -- argumento
```

Argumentos são encaminhados sem interpretação por shell. O status de saída do processo é preservado.

### 6. `pkg migrate`
Captura um perfil antigo que ainda usa a ativação legada em uma geração marcada como **não verificada**. Os artefatos precisam ser reacquiridos e replanejados para obter uma geração verificada.

### 7. `pkg list`
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

### 5. `pkg info`
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

### 6. `pkg repo sync`
Sincroniza o catálogo local com os repositórios remotos configurados.

```bash
pkg repo sync
# aliases compatíveis:
pkg sync
pkg repo update
```

`pkg update` atualiza os pacotes instalados e é um alias de `pkg upgrade`.

**Comportamento:**
- Lê a lista de repositórios declarada em `repositories.toml`. Se o arquivo não existir, cria um arquivo padrão contendo os repositórios do **Ubuntu Noble** e **Debian Bookworm**.
- Baixa o arquivo `InRelease` de cada repositório e **valida criptograficamente sua assinatura GPG**.
- Baixa os índices (`Packages.xz` ou `Packages.gz`) e confere tamanho e SHA-256 contra o conteúdo autenticado do `InRelease` antes de descompactá-los. O fallback gzip também precisa estar listado no conteúdo assinado.
- Executa um commit atômico no banco de dados SQLite local, substituindo o catálogo antigo pelo novo snapshot.
- Se o download ou validação falhar, o catálogo anterior permanece ativo sem corrupção.

---

### 7. `pkg search`
Realiza buscas instantâneas por pacotes disponíveis no catálogo local atualizado.

```bash
pkg search ripgrep
pkg search discord
```

**Comportamento:**
- Não faz requisições de rede. Consulta diretamente os índices indexados no SQLite (`remote_packages`), retornando resultados em sub-milissegundos.

---

## Upgrade, diagnóstico e perfis

O comando upgrade calcula candidatos mais novos no snapshot local e aplica a
atualização em gerações sucessivas. Dependências declaradas são adquiridas
antes do pacote principal; se uma etapa falhar, a geração ativa original é
restaurada.

    pkg upgrade --dry-run
    pkg upgrade nome-do-pacote
    pkg upgrade --jobs 4
    pkg upgrade --json
    pkg update

`--jobs <N>` limita de 1 a 16 as aquisições concorrentes de artefatos
(padrão: 4). Downloads e verificações de cache usam tarefas Tokio; instalação,
SQLite e a publicação de geração continuam seriais e determinísticos.

O comando doctor executa diagnósticos somente leitura sobre transações, banco,
store, perfis, manifests de runtime e objetos inalcançáveis. O relatório
classifica achados como OK, WARN, ERROR, RECOVERABLE ou
MANUAL_ACTION_REQUIRED. Para reconciliar explicitamente transações conhecidas,
use `pkg doctor --repair`; combine com `--all` para inspecionar todos os perfis.
O modo de reparo nunca remove conteúdo de propriedade incerta.

### `pkg integrate` e `pkg deintegrate`

Esses comandos ativam ou removem explicitamente entradas `.desktop`, ícones e
descrições MIME suportadas na raiz de dados do usuário. Cada link é registrado
com o pacote, objeto do store, digest da origem, tipo e destino. Conflitos ou
substituições feitas pelo usuário interrompem a operação sem apagar o arquivo.

```bash
pkg integrate nome-do-pacote --dry-run
pkg integrate nome-do-pacote
pkg deintegrate nome-do-pacote
```

As operações não executam scripts de mantenedor, não atualizam bancos de dados
do sistema e não escrevem em caminhos globais. `pkg remove` aplica a mesma
verificação de propriedade e desfaz integrações pertencentes ao pacote.

Perfis de tarefa podem ser criados e removidos sem tocar nos objetos
compartilhados:

    pkg profile create tarefa
    pkg profile list
    pkg profile drop tarefa

O descarte recusa conteúdo regular não reconhecido; objetos compartilhados
ficam no store para coleta posterior pelo gc. O comando query-command encontra
provedores instalados no perfil. O comando mcp oferece esses contratos por
JSON-RPC sobre stdin/stdout, incluindo busca, instalação, remoção, consulta,
diagnóstico e perfis.

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

### `pkg repo sync`
Sincroniza os repositórios remotos configurados. `pkg sync` e `pkg repo update`
são aliases compatíveis:

```bash
pkg repo sync
```

### `pkg repo add`
Adiciona um novo repositório ao arquivo `repositories.toml`:

```bash
# Sintaxe: pkg repo add <ID> <URL> <DISTRIBUIÇÃO> [COMPONENTES...]
pkg repo add debian-sid http://deb.debian.org/debian sid main contrib non-free
```
Após adicionar, basta rodar `pkg repo sync` (ou um alias) para indexar os novos pacotes.

## Concorrência e recuperação

A inicialização adquire o lock exclusivo antes de abrir o banco e recuperar transações. Instalação, remoção, download para o cache e atualização do catálogo usam o mesmo lock. Enquanto outro escritor estiver ativo, um novo comando, inclusive `list`, pode retornar `LockError`; ele não tenta recuperar uma transação ainda em andamento.

Perfis usam nomes compostos por letras ASCII, números, ponto, hífen e sublinhado; nomes vazios, `.` e `..` não são aceitos como alvos de instalação ou remoção.
