# Plano de correção do code review

Status: concluído. Os oito problemas foram corrigidos, preservando as alterações locais existentes.

1. **Isolamento de arquivos:** validar versões e caminhos do store; extrair arquivos sem seguir links; verificar cadeias de links antes da promoção. Regressões: versão com traversal, links encadeados, entradas duplicadas e links internos válidos.
2. **Integridade remota:** vincular os índices aos hashes e tamanhos do InRelease verificado; revalidar o cache e publicar downloads apenas depois da verificação. Regressões: índices adulterados, cache divergente, interrupção de download e caminhos de digest inválidos.
3. **Estado e perfis:** adquirir o lock antes da recuperação; preservar objetos referenciados por outros perfis e evitar substituição de registros com cascata. Regressões: inicialização com escritor ativo, reinstalação e remoção entre perfis.
4. **Ativação e compatibilidade:** verificar propriedade dos links, preservar arquivos do usuário e rejeitar ELF inválido ou dependências ausentes antes da promoção. Regressões: conflitos no disco, atualizações legítimas, arquivos substituídos pelo usuário e ELF incompatível.
5. **Validação final:** atualizar os contratos afetados, executar testes locais determinísticos, `cargo fmt --check`, `cargo check` e Clippy com avisos tratados como erro.

As alterações não ampliam o suporte a formatos nem implementam resolução completa de dependências/ABI. Os testes devem verificar tanto rejeições quanto instalações válidas.

## Resultado da validação

- 53 testes passaram, sem testes ignorados; 15 testes adicionais em relação aos 38 originais.
- Os testes antigos de cache e limite de download passaram a verificar efetivamente os erros; o teste de assinatura não depende mais de arquivos opcionais em `/tmp`.
- As sete reproduções locais do review foram repetidas: arquivos externos preservados, instalações inválidas rejeitadas e segundo perfil funcional após remoção no primeiro.
- Índices xz/gzip válidos e assinados foram aceitos. Índices comprimidos válidos com conteúdo adulterado e assinaturas alteradas foram rejeitados, preservando o catálogo anterior.
- `cargo fmt --all --check`, `cargo check --workspace --all-targets --offline`, `cargo clippy --workspace --all-targets --offline -- -D warnings` e `cargo test --workspace --all-targets --offline` passaram.
- Reutilizar um objeto compartilhado também compara seu conteúdo com o staging para impedir a aceitação de payload previamente modificado.

Compatibilidade documentada: links pendentes/cíclicos são rejeitados; comandos podem retornar erro de lock enquanto houver um escritor ativo. A inspeção ELF rejeita erros de parsing e bibliotecas ausentes, mas não implementa resolução completa de versões de símbolos/ABI.

## Correções da revisão seguinte

Também foram corrigidos os casos encontrados na segunda revisão: recuperação restaura links após falha de upgrade; o estado de instalação é gravado em uma transação SQLite; binários removidos por uma nova versão são desativados; membros `data.tar.*` duplicados são rejeitados; a seleção remota exige nome exato; RPATH/RUNPATH e `$ORIGIN` entram na resolução ELF; bibliotecas apenas armazenadas em `usr/lib` sem um caminho efetivo não são tratadas como disponíveis; e IDs de transação inválidos não podem escapar do diretório de staging.
