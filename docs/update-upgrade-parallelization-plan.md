# Plano de implementação: `pkg update` / `pkg upgrade` paralelizáveis

Data: 2026-09-18  
Status: etapas de aquisição implementadas; transação de lote e staging paralelo pendentes.

## Estado da implementação (2026-09-18)

`update` e `upgrade` aceitam `--jobs` (1--16, padrão 4), deduplicam por
digest, usam `JoinSet` e `Semaphore`, e executam a aquisição fora do `Engine`.
Hashing, verificação de cache e publicação do artefato usam o pool bloqueante;
downloads permanecem assíncronos. Raízes e cada fronteira de dependências já
descoberta são adquiridas concorrentemente, mas a expansão do grafo, a
instalação e a publicação da geração continuam seriais.

Uma transação única para toda a closure do upgrade e a preparação paralela de
staging continuam como trabalho das etapas 5 e 6. A implementação atual
preserva o journal unitário existente e o rollback da geração anterior em caso
de falha de aplicação.

## Objetivo

Permitir que `pkg update` e `pkg upgrade` adquiram, verifiquem e façam
preflight de vários artefatos em paralelo. `pkg update` permanece um alias de
`pkg upgrade`; ambos devem usar o mesmo executor.

A paralelização não pode alterar a semântica visível: o resultado precisa ser
determinístico, um perfil nunca pode expor uma geração parcial e uma falha de
aquisição não pode modificar estado instalado.

## Estado atual

O comando percorre os upgrades em série:

```text
plano de upgrades
  -> baixa raiz
  -> descobre/baixa dependências
  -> instala dependências
  -> instala raiz
  -> próximo upgrade
```

O cache já publica por digest de modo seguro para corridas, mas a chamada atual
mantém `Engine` e banco no caminho de cada download. A instalação usa
`ProcessLock` e atualiza staging, geração, ativações e SQLite. Essas mutações
não devem ser executadas por tarefas concorrentes.

## Decisão de fronteira

Usar `tokio::spawn` somente para trabalhos com dados próprios e sem estado
mutável do perfil:

| Trabalho | Paralelo | Mecanismo |
|---|---:|---|
| Download HTTP para cache por digest | Sim | `tokio::spawn` + `JoinSet` + `Semaphore` |
| Verificação de tamanho/digest e publicação no cache | Sim, por artefato | tarefa de aquisição |
| Parsing, hash e preflight que bloqueiem CPU/FS | Sim, quando isolados | `tokio::task::spawn_blocking` |
| Expansão do grafo, deduplicação e escolha de candidato | Não | coordenador único |
| Journal, staging promovido, ativação, geração, SQLite e rollback | Não | executor único sob `ProcessLock` |

Não mover `&Engine` para `tokio::spawn`. O banco SQLite e o layout pertencem
ao coordenador; cada tarefa recebe uma especificação imutável com candidato,
caminho de cache, digest, tamanho e limites esperados. Isso evita compartilhar
uma conexão SQLite entre tarefas e torna o limite de concorrência explícito.

## Modelo de execução desejado

```text
snapshot de estado + catálogo
          |
          v
coordenador cria UpgradeWorkPlan determinístico
          |
          v
JoinSet de aquisição limitada (jobs = N)
  ├─ download/verificação/cache de A
  ├─ download/verificação/cache de B
  └─ download/verificação/cache de C
          |
          v
coordenador parseia metadados autenticados e expande dependências
          |
          v
todos os artefatos e a closure foram verificados?
          |
          +-- não -> cancelar tarefas; nenhum perfil foi alterado
          |
          +-- sim -> revalidar snapshot e aplicar uma transação de upgrade
                         sob um único ProcessLock
                                  |
                                  v
                         publicar uma geração completa ou restaurar a anterior
```

O coordenador é dono do grafo. Um artefato adquirido pode revelar dependências
que não estavam completas no snapshot remoto; elas entram na fila depois do
parse do artefato autenticado. Assim, a concorrência não volta a depender de
índices incompletos e preserva a correção da instalação transitiva.

## Contratos novos

### `UpgradeWorkPlan`

Estrutura interna, serializável para diagnóstico, contendo:

- snapshot de repositório e revisão do estado local usados no planejamento;
- raízes solicitadas e candidatos exatos;
- mapa de artefatos por digest e seus consumidores;
- estado de cada item: `Pending`, `Downloading`, `Cached`, `Verified`,
  `PreflightFailed`, `Cancelled` ou `Ready`;
- closure de dependências e ordem topológica de aplicação;
- limites usados (`jobs`, artefatos, bytes e profundidade);
- causas encadeadas de falha.

O identificador de trabalho é formado pela identidade dos snapshots, perfil,
raízes e política. Ele não substitui o digest do artefato nem o journal da
transação de instalação.

### `ArtifactAcquisitionSpec`

Extrair do `Engine` uma entrada própria para uma tarefa:

```rust
struct ArtifactAcquisitionSpec {
    candidate: RemotePackage,
    cache_path: PathBuf,
    expected_digest: ArtifactDigest,
    expected_size: u64,
    limits: DownloadLimits,
}
```

Ela baixa para um temporário no diretório do cache, confere tamanho e digest,
sincroniza o arquivo e publica sem sobrescrever. Quando outra tarefa ou
processo vencer a corrida pelo mesmo digest, a tarefa revalida o arquivo
publicado antes de aceitá-lo.

### Eventos de progresso

Cada tarefa envia eventos por um canal limitado ao coordenador:

```text
Queued | Started | BytesDownloaded | CacheHit | Verified | Failed | Cancelled
```

O coordenador é a única parte que escreve na interface humana. No modo normal
mantém múltiplas barras sem intercalar linhas; em `--json`, não escreve
progresso em stdout e emite somente o documento final determinístico.
`tracing` recebe `work_id`, digest, pacote, repositório e tentativa.

## Etapas de implementação

### 1. Fixar semântica e limites

**Arquivos principais:** `context/commands/update.md`,
`context/commands/upgrade.md`, `context/design/transaction-model.md`,
`crates/pkg-cli/src/main.rs`.

- Definir que a concorrência padrão é de aquisição, não de publicação.
- Adicionar `--jobs <N>` a `update` e `upgrade`. Começar com padrão `4`,
  faixa limitada e `--jobs 1` como referência serial reproduzível.
- Definir limites globais para artefatos, bytes anunciados e profundidade da
  expansão. Dependências descobertas não podem criar tarefas ilimitadas.
- Manter `--dry-run` sem download e, portanto, sem iniciar tarefas.
- Documentar interrupção: artefatos verificados podem ficar no cache; store,
  geração e registros instalados não são publicados antes do commit.

**Aceite:** `update`, `upgrade` e variantes com nome aceitam a mesma política;
`--jobs 1` preserva o comportamento serial de referência.

### 2. Separar aquisição do `Engine`

**Arquivos principais:** `crates/pkg-core/src/engine.rs`, transporte/cache e
testes de download.

- Extrair aquisição para uma API que receba `ArtifactAcquisitionSpec`, sem
  consultar SQLite nem mutar perfil.
- Preservar verificação de digest, tamanho, tipo de arquivo do cache e
  publicação `persist_noclobber`.
- Mover hash e I/O de arquivo bloqueantes para `spawn_blocking`, mantendo HTTP
  na tarefa assíncrona.
- Retornar resultado tipado com caminho, `CacheHit` e evidência de verificação.
- Cobrir solicitações duplicadas dentro do processo e entre processos.

**Aceite:** a aquisição pode rodar em tarefa independente sem reter `Engine`;
cache corrompido e corrida pelo mesmo digest continuam seguros.

### 3. Implementar coordenador concorrente

**Arquivos principais:** novo módulo de upgrade em `pkg-core` ou adaptador na
CLI, `crates/pkg-cli/src/main.rs`.

- Criar `JoinSet<AcquisitionResult>` e `Semaphore` com `N` permissões.
- Deduplicar antes de criar tarefas por digest; consumidores diferentes do
  mesmo conteúdo compartilham trabalho.
- Manter fila determinística por identidade de pacote/digest. A ordem de
  conclusão da rede não define resolução nem publicação.
- No primeiro erro permanente, parar de agendar, chamar `abort_all`, coletar
  `JoinError` e devolver a causa ligada ao pacote solicitante.
- Propagar cancelamento do processo ao `JoinSet`; cache já verificado não é
  removido.

**Aceite:** com servidor de teste retardado, `--jobs 3` tem no máximo três
downloads em voo, `--jobs 1` tem um e os dois modos produzem o mesmo plano.

### 4. Expandir dependências durante aquisição

**Arquivos principais:** resolvedor, planner, adaptadores e coordenador.

- Semear o grafo com as raízes do upgrade.
- Depois de verificar cada artefato, parsear seus metadados e replanejar sua
  closure contra o snapshot congelado.
- Inserir dependências novas somente se a identidade não estiver concluída,
  agendada ou em progresso.
- Manter ordenação topológica; ciclos que exigirem bibliotecas antes da
  publicação falham com diagnóstico até existir preparação de lote suficiente.
- Tratar `Breaks`/`Conflicts` contra todos os pacotes substituídos pela closure,
  não apenas as raízes nomeadas pelo usuário.

**Aceite:** um índice sem metadados transitivos ainda instala
`raiz -> intermediário -> provedor`; atrasos de rede não mudam a ordem final.

### 5. Criar transação de upgrade em lote

**Arquivos principais:** engine, planner, activation, state e recovery.

- Substituir o laço CLI de `install_with_options_replacing` por uma operação de
  lote no core.
- Depois de adquirir tudo, obter o `ProcessLock` uma vez, reconciliar e
  revalidar versões instaladas e snapshots. Se o estado mudou durante fetch,
  abortar antes de mutar e replanejar; cache pode ser reutilizado.
- Registrar um journal único com raízes, closure, objetos criados/reutilizados
  e geração anterior.
- Preparar a geração inteira antes de trocar o ponteiro do perfil e reconciliar
  o banco. Recovery escolhe somente a geração anterior ou a publicada.
- Remover o rollback melhor-esforço por item quando o executor de lote cobrir
  a mesma fronteira. A instalação unitária continua para `pkg install`.

**Aceite:** falha antes do commit não altera o perfil; falhas em cada fronteira
do journal são recuperáveis; mudança externa durante fetch não mistura closures.

### 6. Paralelizar preparação somente após o lote existir

**Arquivos principais:** engine, formatos, runtime e transação.

- Identificar trabalho com staging exclusivo: extração, inventário,
  transformações puras e inspeção ELF.
- Rodar apenas essas partes em `spawn_blocking`, com limite separado de CPU/FS
  e diretórios de staging distintos.
- Não promover, ativar, gravar SQLite nem materializar `profile/lib` em tarefas.
- O coordenador valida resultados, colisões de comando e referências de runtime
  antes da promoção serial e publicação da geração.

**Aceite:** dois pacotes preparam em paralelo sem compartilhar staging; uma
falha limpa somente seu staging e mantém a geração anterior ativa.

### 7. Expor resultado e operar a mudança

**Arquivos principais:** CLI, documentação, observabilidade e testes de
processo.

- Exibir itens baixados, vindos do cache, preparados e aplicados, sem tornar a
  serialização interna uma API pública por acidente.
- Adicionar campos estruturados para concorrência, bytes, duração e primeira
  falha, com versão explícita de schema se necessário.
- Medir `--jobs 1`, padrão e valores maiores em repositório controlado,
  distinguindo cache frio, cache quente e publicação.
- Atualizar os comandos para manter `pkg repo sync` separado da atualização de
  pacotes.

**Aceite:** saída humana não se embaralha, JSON tem documento único e a
documentação não promete paralelismo de mutações de perfil.

## Matriz mínima de testes

| Caso | Prova esperada |
|---|---|
| `--jobs 1` | Mesma closure e geração do fluxo serial atual. |
| Limite de concorrência | Fixture mede requisições em voo `<= N` e `> 1` quando aplicável. |
| Mesmo digest | Uma aquisição válida; consumidores compartilham cache verificado. |
| Índice transitivo incompleto | Metadados do artefato encontram e baixam o provedor antes do consumidor. |
| Falha de aquisição | Tarefas são canceladas; nenhuma geração é publicada; causa raiz preservada. |
| Interrupção | Temporários não são publicados; cache verificado sobrevive; reinício reutiliza cache. |
| Estado muda durante fetch | Revalidação aborta antes da mutação e pede replanejamento. |
| Respostas fora de ordem | Ordem de aplicação e JSON são determinísticos. |
| Conflito/versionamento | Closure substitui biblioteca antiga sem conflito contra sua geração anterior. |
| Falha de journal | Recovery deixa apenas geração anterior ou geração completa visível. |

## Riscos e decisões pendentes

- `tokio::spawn` não torna hash ou I/O de arquivo não bloqueante; esses trechos
  precisam de `spawn_blocking` e limites próprios.
- Publicar pacote ao terminar o download seria rápido, mas quebraria a fronteira
  de geração completa; fica fora do escopo.
- Ciclos de runtime e bibliotecas cruzadas podem exigir preparação de lote antes
  que qualquer provedor seja ativado.
- Muitos downloads pressionam espelhos e disco; o padrão conservador e limites
  configuráveis são parte do contrato.

## Condição de conclusão

O trabalho estará concluído quando `update` e `upgrade` compartilharem o mesmo
executor, downloads concorrentes forem limitados e deduplicados por digest,
dependências descobertas em artefatos forem adquiridas antes dos consumidores,
e toda mutação de estado continuar publicada por uma única transação de geração
recuperável.
