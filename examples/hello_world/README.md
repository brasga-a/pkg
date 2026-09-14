# Exemplo: Hello World (`hello-world`)

Este diretório contém um pacote de exemplo `.deb` simples (`hello-world_1.0.0_amd64.deb`) projetado para demonstrar e testar o funcionamento do gerenciador de pacotes `pkg` (Milestone 1).

## Estrutura do Exemplo

```
examples/hello_world/
├── README.md                      # Instruções e comandos de teste
├── build.sh                       # Script para recompilar o pacote .deb
├── hello-world_1.0.0_amd64.deb    # Pacote Debian compilado e pronto para teste
└── package/                       # Árvore fonte do pacote
    ├── DEBIAN/
    │   └── control                # Metadados do pacote (nome, versão, arquitetura, etc.)
    └── usr/
        └── bin/
            └── hello-world        # Script executável fornecido pelo pacote
```

---

## Passo a Passo para Testar com o `pkg`

Todos os comandos abaixo podem ser executados a partir da raiz do projeto (`/home/brasga/projects/pkg`).

### 1. Inspecionar os Metadados do Pacote (`pkg info`)

Analisa o arquivo `.deb` de forma segura (sem usar o `dpkg` do sistema operacional) e exibe os metadados normalizados:

```bash
cargo run --bin pkg -- info examples/hello_world/hello-world_1.0.0_amd64.deb
```

*Saída esperada:*
```text
Identity:
  Name:         hello-world
  Version:      1.0.0
  Architecture: x86_64
  Format:       deb
  Description:  Hello World example package for pkg
A simple hello world demonstration package for testing pkg
package installation, profile activation, and removal.

Provenance:
  Digest:       sha256:...
  Artifact size: ... bytes

Provides:
  - bin:hello-world
```

---

### 2. Simular a Instalação com Dry-Run (`pkg install --dry-run`)

Valida integridade, verifica conflitos e simula o plano de transação sem alterar nenhum arquivo em disco:

```bash
cargo run --bin pkg -- install examples/hello_world/hello-world_1.0.0_amd64.deb --dry-run
```

*Saída esperada:*
```text
Dry-run mode: no changes will be made to the system.

Planned actions for profile 'default':
  [+] Install hello-world 1.0.0 (deb)
      Store path: /home/.../.local/share/pkg/store/...-hello-world-1.0.0
      Binaries:
        - hello-world -> profiles/default/bin/hello-world

Summary:
  Packages to install: 1
  Packages to remove:  0
  Estimated disk size: ... bytes
```

---

### 3. Instalar o Pacote (`pkg install`)

Extrai o payload de forma segura para a store isolada (`~/.local/share/pkg/store/`), registra o estado no banco SQLite transacional e cria o symlink do executável no profile ativo:

```bash
cargo run --bin pkg -- install examples/hello_world/hello-world_1.0.0_amd64.deb
```

*Saída esperada:*
```text
Extracting payload for hello-world 1.0.0...
Installing hello-world 1.0.0 into profile 'default'...
Activated 1 executable(s):
  - hello-world
Successfully installed hello-world 1.0.0
```

---

### 4. Listar os Pacotes Instalados (`pkg list`)

Exibe todos os pacotes atualmente instalados no perfil:

```bash
cargo run --bin pkg -- list
```

*Saída esperada:*
```text
Profile: default
NAME         VERSION  ARCH    FORMAT  STORE ID
hello-world  1.0.0    x86_64  deb     ...-hello-world-1.0.0
```

---

### 5. Executar o Binário Instalado

Execute o comando instalado diretamente através do profile ativo:

```bash
~/.local/share/pkg/profiles/default/bin/hello-world
```

*Saída esperada:*
```text
Hello, World from pkg!
```

> **Dica**: Você pode adicionar o diretório de perfis ao seu `PATH` para rodar comandos diretamente:
> ```bash
> export PATH="$HOME/.local/share/pkg/profiles/default/bin:$PATH"
> hello-world
> ```

---

### 6. Remover o Pacote (`pkg remove`)

Remove os symlinks de ativação do profile, apaga o diretório isolado da store e atualiza o banco de dados transacional:

```bash
cargo run --bin pkg -- remove hello-world
```

*Saída esperada:*
```text
Deactivating hello-world from profile 'default'...
Removed 1 executable symlink(s).
Pruned store object: /home/.../.local/share/pkg/store/...-hello-world-1.0.0
Successfully removed hello-world
```

---

### 7. Confirmar a Remoção (`pkg list`)

Verifique se o pacote foi completamente desinstalado:

```bash
cargo run --bin pkg -- list
```

*Saída esperada:*
```text
Profile: default
No packages installed in profile 'default'.
```

---

## Como Reconstruir o Pacote `.deb`

Se você fizer alterações no script `package/usr/bin/hello-world` ou nos metadados em `package/DEBIAN/control`, basta rodar o script auxiliar:

```bash
./examples/hello_world/build.sh
```
