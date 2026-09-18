# Catálogo Remoto e Confiança Criptográfica (Milestone 2)

O **Milestone 2** introduz no `pkg` a capacidade de sincronizar catálogos remotos e baixar artefatos sob demanda, mantendo padrões rigorosos de segurança e integridade criptográfica.

---

## 🏛️ O Modelo de Estado Duplo (Dual-State Architecture)

Para evitar os problemas de configurações em "caixa preta" e ao mesmo tempo garantir a máxima velocidade de execução, o `pkg` adota uma separação estrita entre **Declaração Humana** e **Estado de Execução**:

```text
       [ Usuário / Admin ]
               │
               ▼ (Edição manual ou `pkg repo add`)
    ~/.local/share/pkg/repositories.toml      <-- Fonte da Verdade (TOML transparente)
               │
                ▼ (Comando: `pkg repo sync`; aliases: `pkg sync`, `pkg repo update`)
      Validação Criptográfica GPG (InRelease)
                │
                ▼ (Commit atômico em transação SQLite)
         ~/.local/share/pkg/state/pkg.db       <-- Snapshot Imutável Indexado
                │
                ▼ (Buscas instantâneas sub-milissegundo)
       `pkg search` / `pkg install`
```

1. **`repositories.toml` (A Fonte da Verdade):**
   - Arquivo simples, legível e versionável em repositórios de *dotfiles*.
   - Pode ser editado com qualquer editor de texto ou manipulado pelos comandos `pkg repo add` e `pkg repo list`.

2. **SQLite Snapshots (Performance Turbo):**
   - Ao executar `pkg repo sync` (ou um alias), os metadados são descompactados e inseridos nas tabelas `repositories` e `remote_packages`.
   - Comandos como `pkg search` ou o resolvedor de nomes de `pkg install` consultam exclusivamente o SQLite, garantindo respostas instantâneas sem overhead de parsing de texto ou tráfego de rede.

---

## 🔐 Confiança Criptográfica e Validação GPG

De acordo com o princípio **INV-005** e **ADR-008**, nenhum metadado de repositório é aceito às cegas.

### Fluxo de Validação do `InRelease`
1. O `pkg` faz o download do arquivo `InRelease` do repositório (ex: `http://deb.debian.org/debian/dists/bookworm/InRelease`).
2. O arquivo `InRelease` é uma mensagem assinada em texto claro (*cleartext signed message*) segundo a especificação OpenPGP (RFC 4880).
3. **Resolução Inteligente de Chaves:**
   - Se um caminho explícito (`public_key_path`) estiver configurado no `repositories.toml`, o `pkg` o utiliza prioritariamente.
   - Caso contrário, o `pkg` busca nos keyrings padrão do sistema hospedeiro (`/usr/share/keyrings/ubuntu-archive-keyring.gpg`, `/etc/apt/trusted.gpg.d/`, etc.).
   - Se o keyring do Debian não estiver instalado no sistema hospedeiro, o `pkg` realiza o download seguro da chave oficial pública para `~/.local/share/pkg/keyrings/debian-<distro>.asc`.
4. **Decodificação Multi-Key e Suporte a Subchaves:**
   - Filtra pacotes privados de confiança do GnuPG (`Tag::Trust`) presentes em arquivos binários `.gpg`.
   - Itera por todas as chaves públicas presentes no chaveiro.
   - Suporta verificação contra a chave primária ou qualquer uma de suas subchaves públicas (`public_subkeys`).
5. Se a assinatura digital for válida:
   ```text
   ✓ Verified GPG signature for http://archive.ubuntu.com/ubuntu (noble) using /usr/share/keyrings/ubuntu-archive-keyring.gpg
   ✓ Verified GPG signature for http://deb.debian.org/debian (bookworm) using ~/.local/share/pkg/keyrings/debian-bookworm.asc
   ```
6. Se a assinatura digital falhar ou for adulterada, a atualização é rejeitada com erro de segurança e o snapshot anterior é preservado intacto.
7. Antes de descompactar cada índice, o `pkg` exige sua entrada na seção `SHA256` do conteúdo assinado e verifica seu tamanho e hash. Um índice válido como gzip/xz, mas com conteúdo diferente, também é rejeitado. O mesmo requisito vale para o fallback de xz para gzip.

O `InRelease` tem limite de 4 MiB; índices comprimidos têm limite de 256 MiB e o texto descomprimido de 1 GiB. Até quatro componentes são baixados simultaneamente.

---

## ⚡ Download Bounded e Streaming Seguro (`BoundedDownloader`)

Para proteger a máquina contra ataques de exaustão de disco ou saturação de memória RAM:
- O módulo `transport/download.rs` utiliza `reqwest` com streaming assíncrono sobre `tokio`.
- **Pre-flight check:** Se o cabeçalho `Content-Length` exceder o limite estabelecido (padrão de 5 GB), o download é abortado antes de iniciar.
- **Streaming em chunks:** O arquivo é gravado diretamente no disco em pequenos blocos sem nunca carregar o pacote inteiro na memória RAM.
- **Controle ativo de quota:** Um contador monitora os bytes recebidos em tempo real. Se o fluxo ultrapassar a cota, a conexão é encerrada imediatamente e o arquivo parcial é excluído do disco.

---

## 🗄️ Cache de Artefatos Endereçado por Digest (INV-017)

Quando um pacote remoto precisa ser instalado, o `pkg` nunca executa o download diretamente para a área de instalação:

```text
Download HTTP ──► arquivo temporário ──► tamanho + SHA-256 ──► cache/artifacts/sha256/<hash> ──► Staging & Install
```

1. **Localização:** Os pacotes baixados são salvos em `~/.local/share/pkg/cache/artifacts/sha256/<digest>`.
2. **Deduplicação:** Um arquivo existente é reaproveitado somente após revalidar seu tamanho e SHA-256. Links simbólicos não são aceitos como entradas do cache.
3. **Verificação de Integridade Pós-Download:**
   - O arquivo no cache é lido e seu digest SHA-256 é recalculado.
   - Se o hash calculado for divergente do declarado no catálogo do repositório, o arquivo é imediatamente apagado do cache e a operação é abortada com `Error::SecurityViolation`.
   - Um digest precisa conter exatamente 64 caracteres hexadecimais antes de ser usado como nome de arquivo.
   - Downloads incompletos não são publicados. O arquivo temporário é removido ao retornar um erro ou cancelar a operação; após interrupção abrupta do processo, qualquer temporário remanescente não é aceito como entrada definitiva do cache.
