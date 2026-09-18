# Plano de implementação: instalação e execução rootless por comando

Data: 2026-09-17

Status: **em implementação; gates de aceitação pendentes**.

Base: [Rootless Execution: Verified Environments per Command](../context/design/rootless-execution-counter-proposal.md).

As combinações comprovadas ficam registradas na [matriz de suporte executável](support-matrix.md).

Este documento substitui o plano anterior de fechamento do `install`. Ele organiza a contraproposta em entregas verificáveis; não a declara aceita nem implementada. A aprovação existente do [Gate M3 — Resolver](../context/roadmap/release-gates.md#gate-m3--resolver) não certifica o novo caminho de instalação e execução.

### Progresso da implementação

Esta entrega já conecta o caminho transversal ao install real: o planner
consulta o resolvedor para restrições declaradas, o `--dry-run` pode abrir uma
raiz vazia sem criar banco ou diretórios, dependências são adquiridas na ordem
da closure, a inspeção ELF registra ABI e rejeita máquinas ou caminhos de
loader incompatíveis, manifests e views de runtime por comando são gravados,
launchers preservam o contrato após a promoção, o runner nativo é um bootstrap
Rust estático com configuração estruturada, gerações e rollback têm ponteiros
recuperáveis, a recuperação adota commits publicados e `pkg gc` analisa/coleta
objetos registrados sem referências. A matriz já prova provedores cross-format,
provedores diferentes com o mesmo SONAME, scripts puros Perl/Node, templates
`.ucf` explícitos e recusa de namespaces gerenciados por symlink. A matriz
determinística já cobre falhas de install, rollback e GC; ainda faltam a matriz
real de ABI/formato/host, receitas para extensões nativas e runtimes além do
adaptador genérico, algumas fronteiras de upgrade e a aprovação formal de G1–G10;
por isso o plano continua pendente. A superfície M5 já inclui pkg upgrade,
pkg doctor, perfis efêmeros, --json, códigos de saída tipados,
pkg query-command e o servidor pkg mcp; cada uma dessas interfaces tem teste
de processo ou de núcleo. `pkg doctor --repair` reconcilia somente transações
pkg conhecidas antes de repetir o diagnóstico, mas isso não amplia a matriz de compatibilidade dos
runtimes.

### Por que a primeira entrega é transversal

Uma instalação só é útil quando o mesmo artefato percorre o caminho inteiro:
resolução, aquisição, staging, verificação, runtime, publicação e recuperação.
Implementar apenas um adaptador de formato ou apenas o runner deixaria as
interfaces críticas sem prova e poderia esconder incompatibilidades até a
execução real. A fatia transversal reduz esse risco com um caso pequeno, mas
executável, e cria contratos que as próximas etapas podem ampliar sem trocar a
semântica no meio do projeto.

Ela não significa que todos os formatos, hosts ou runtimes já estejam cobertos.
Significa que cada fronteira principal tem uma implementação mínima integrada e
um teste que mostra onde a cobertura termina. As extensões de suporte entram
depois como novas combinações da matriz, com suas próprias evidências.

## 1. Resultado esperado

`pkg install` deve adquirir e verificar os artefatos necessários, produzir um plano completo e publicar comandos com ambientes de execução definidos. Cada comando deve identificar seu executável ou interpretador, seus provedores de bibliotecas e módulos, as adaptações aplicadas e o conjunto completo de dependências diretas e transitivas, chamado aqui de **closure**.

Um aplicativo RPM do Fedora poderá reutilizar uma biblioteca de um pacote Debian do Ubuntu quando houver evidência suficiente de compatibilidade. O mesmo contrato se aplica a pacotes ALPM, incluindo `.pkg.tar.zst`. O formato e a distribuição de origem identificam como obter e interpretar o artefato; não comprovam nem impedem, por si sós, a compatibilidade de execução. Cada combinação anunciada como suportada exige testes reais.

O suporte permanece limitado a aplicações executadas com as permissões do usuário. O isolamento do store não é uma sandbox de processos. Não fazem parte deste plano substituir componentes críticos do host, modificar bancos de `apt`/`dnf`/`pacman`, escrever payloads em `/usr` ou executar scripts de mantenedor automaticamente.

## 2. Contratos que orientam as entregas

| Área | Contrato a implementar |
|---|---|
| Planejamento | Função pura sobre artefatos verificados e snapshots de catálogo, host, estado e política; sem rede, extração temporária, criação de diretórios, inicialização de banco ou lock de escrita. |
| Aquisição | Serviço separado obtém entradas ausentes, verifica integridade e proveniência e volta a chamar o planejador. Impor limites de rodadas, artefatos, bytes e tamanho do grafo. |
| `--dry-run` | Usa o mesmo planejador, sem aquisição nem escritas. Entradas ausentes produzem prévia incompleta, evidências faltantes e resultado diferente de sucesso para um plano solicitado como verificado. |
| Dependências | Resolução por capacidades e evidência do provedor exato. Preservar restrições e regras de versão da origem; nenhuma equivalência automática entre nomes Debian, RPM e ALPM. |
| Bibliotecas | Visão imutável por ambiente de comando, com referências aos arquivos selecionados. Nenhum novo diretório global `profile/lib` ou exportação global de caminhos de bibliotecas. |
| Adaptações | Receitas explícitas e versionadas, planejadas antes do staging, com precondições, saídas exatas e pós-condições. Requisitos obrigatórios desconhecidos bloqueiam a ativação suportada. |
| Integridade | Distinguir digest do artefato, identidade da derivação e digest da árvore realizada após as transformações. |
| Publicação | Preparar uma geração completa, trocar o ponteiro do perfil e reconciliar o banco por protocolo durável de recuperação. Filesystem e SQLite não formam uma única transação atômica. |
| Retenção | Preservar objetos referenciados por runtimes, gerações retidas e transações incompletas. Não fazer coleta física automática nesta primeira implementação. |

O plano adota o `--dry-run` sem downloads da contraproposta. A abordagem anterior de baixar para um diretório temporário deixa de fazer parte deste plano. A formalização nos contratos canônicos e ADRs é uma entrega da etapa 0.

## 3. Escopo e relação com os milestones

| Frente | Relação com este plano |
|---|---|
| M3 — resolução | Reutilizar adaptadores e resolvedor, enriquecer evidências e conectá-los ao caminho real de instalação. Não reabrir ou ampliar a certificação do Gate M3 sem alterar seu escopo explicitamente. |
| Instalação e runtime | Entregar planos completos, bibliotecas por comando, runner, receitas e provas de execução. É o foco das etapas 1–6. |
| M4 — integração desktop | Continua separado. Um requisito obrigatório de integração não suportado impede a ativação; uma omissão opcional exige classificação explícita por receita e diagnóstico. |
| M5 — gerações e recuperação | Publicação por geração, retenção e migração nas etapas 7–8 são dependências do contrato completo da contraproposta e devem ser coordenadas com o M5. |
| M5 — rollback e GC | Preservar closures de gerações anteriores e testar restauração. Comando público de rollback e coleta física exigem entregas próprias; GC depende também de um modelo de tempo de vida dos processos/runtimes. |

Nenhuma etapa isolada permite anunciar que toda a instalação está fechada. Enquanto houver ativação legada por entrada individual, documentar essa semântica sem prometer troca atômica do perfil inteiro. Python é a primeira família com módulos nativos verificados; scripts puros Perl e Node já usam o adaptador genérico quando o intérprete do host está disponível, enquanto R, Ruby e extensões nativas precisam de receitas e gates próprios.

## 4. Sequência de implementação

### Etapa 0 — Fixar decisões e critérios de suporte

**Dependências:** nenhuma. **Entrega:** contratos reconciliados e matriz inicial de validação.

- Atualizar a inspeção do checkout e registrar quais caminhos já existem e quais continuam pendentes. O inventário da contraproposta é uma referência de código, não um resultado de testes.
- Definir os hosts de teste por arquitetura, libc/loader e versões suportadas. Manter a execução por loader explícito desabilitada até comprovar sua semântica nessa matriz; outros loaders precisam de adaptadores próprios.
- Resolver a restrição ampla do [modelo cross-distro](../context/design/cross-distro-installation.md) sobre pacotes que contêm `/lib` ou `/usr/lib`: permitir provedores de bibliotecas de aplicação no store requer uma decisão canônica precisa, preservando a proibição de substituir componentes críticos do sistema.
- Formalizar o contrato de `--dry-run`, os resultados de compatibilidade e a publicação por geração no [registro de decisões](../context/meta/decisions.md) e nos ADRs afetados. Reconciliar os modelos de [store](../context/design/store-layout.md), [filesystem](../context/design/filesystem-layout.md), [transação](../context/design/transaction-model.md), [compatibilidade](../context/design/compatibility-model.md) e [banco](../context/design/database-state.md), além do modelo original e do contrato de instalação.
- Definir como os novos resultados aparecerão na CLI e na saída estruturada, conforme a política pública de schema. Não transformar a serialização interna dos planos em API pública estável por acidente.

**Aceite:** a matriz contém tuplas concretas de host/runtime e combinações de formatos a testar; políticas e ADRs não contradizem o comportamento previsto. A contraproposta só muda de status quando houver decisão registrada, não pela existência deste plano.

### Etapa 1 — Tornar os resultados do comando confiáveis

**Dependências:** contrato da etapa 0. **Arquivos principais:** [CLI](../crates/pkg-cli/src/main.rs), [engine](../crates/pkg-core/src/engine.rs), [contrato de install](../context/commands/install.md), [referência da CLI](cli-reference.md).

- Propagar falhas de preflight e instalação de dependências, preservando suas causas. Substituir os caminhos que descartam esses resultados; imprimir sucesso somente após conclusão confirmada da operação.
- Fazer qualquer falha em alvos solicitados produzir status final diferente de zero. Mostrar quais alvos independentes foram instalados, ignorados ou falharam; não prometer atomicidade entre alvos independentes. A closure de cada alvo será uma unidade recuperável na etapa 7.
- Impedir ativação suportada com requisitos obrigatórios não resolvidos. `--ignore-missing-libs`, se mantido como opção experimental, deve persistir e exibir a evidência incompleta, sem rotular o resultado como verificado.
- Ajustar a documentação de URL explícita ao suporte real: não prometer `pkg install <url>` enquanto não existir contrato próprio de proveniência e verificação. Usar arquivo local ou candidato de repositório neste plano.

**Aceite:** falha de dependência nunca recebe mensagem de sucesso, o alvo dependente não é ativado e a CLI expõe uma cadeia causal útil. Erro parcial de múltiplos alvos é reproduzido em teste de processo.

### Etapa 2 — Criar os contratos de evidência, plano e identidade

**Dependências:** etapa 0. **Arquivos principais:** [domínio](../crates/pkg-core/src/domain), [adaptadores de formato](../crates/pkg-core/src/format), [banco](../crates/pkg-core/src/state/db.rs), [layout do store](../crates/pkg-core/src/store/layout.rs).

Implementar os seguintes contratos internos, evoluindo os módulos existentes antes de criar novas crates:

| Contrato | Conteúdo obrigatório |
|---|---|
| `ArtifactEvidence` | Identidade da origem e do snapshot de catálogo, formato/ecossistema, arquitetura, tamanho, digest e evidência de confiança. Proveniência local sem assinatura permanece distinguível. |
| `PayloadManifest` | Todas as entradas, tipos, modos, links, digests, metadata estática de executáveis/scripts, inventário de requisitos de ciclo de vida e completude da inspeção, com limites de leitura. |
| `AdaptationPlan` | Receita e versão, precondições, parâmetros, arquivos transformados/gerados e pós-condições. |
| `ExecutionPlan` | Comando, executável/interpretador, estratégia de lançamento, argumentos, ambiente, closure, helpers e plugins admitidos. |
| `ProviderEvidence` | Requisito do consumidor ligado à identidade do objeto e caminho relativo do arquivo, ou identidade no host, com proprietário, evidências de ABI e motivo da seleção. |
| `RuntimeManifest` | Plano congelado, referências às visões de bibliotecas/módulos, versão do runner, fatos do host e grafo de referências. |
| `ActivationGeneration` | Conjunto completo de comandos e manifests do perfil, ownership e referência à geração anterior. |
| `TransactionReceipt` | Identidade do plano, gerações anterior/nova, objetos criados ou reutilizados, digests esperados e progresso recuperável. |

- Inspecionar arquivos locais ou em cache em leitura, por parsing estático/streaming limitado; não executar payloads nem extrair para disco para descobrir requisitos. Aplicar limites de entradas, bytes descomprimidos, strings e profundidade dos grafos.
- Calcular a identidade de derivação a partir do artefato, arquitetura, versões de normalizadores/receitas, política e parâmetros, incluindo prefixos absolutos incorporados à saída. Depois de realizar a árvore, calcular seu digest canônico com entradas ordenadas, tipos, modos normalizados, conteúdo e alvos literais de symlinks; registrar a origem dos arquivos gerados e as transformações.
- Evitar identidades circulares: determinar o caminho do store pela derivação antes de renderizar caminhos absolutos; calcular o digest final depois. Identificar runtimes por referências lógicas antes de gerar caminhos próprios. Referências entre pacotes pertencem ao runtime, não à transformação do payload.
- Versionar schema e manifests, manter origem e digests distintos e preparar a representação de instalações legadas como evidência não verificada. Migrar o schema sem apagar snapshots ou registros existentes; a migração dos perfis é tratada na etapa 8.

**Aceite:** planos e manifests são serializáveis internamente e determinísticos; toda saída tem origem e proprietário; artefatos malformados falham de forma limitada. Mudanças de receita ou prefixo não reutilizam silenciosamente uma realização antiga.

### Etapa 3 — Fornecer catálogo, aquisição e fatos do host suficientes

**Dependências:** contratos da etapa 2. **Arquivos principais:** [repositórios](../crates/pkg-core/src/repository), [download](../crates/pkg-core/src/transport/download.rs), [host](../crates/pkg-core/src/host/mod.rs), [banco](../crates/pkg-core/src/state/db.rs).

- Enriquecer candidatos remotos com dependências, alternativas, capacidades fornecidas, conflitos, expressões originais e proveniência. Cobrir campos Debian, RPM-MD e ALPM preservando semântica e comparadores nativos; campos obrigatórios não interpretáveis permanecem explicitamente não resolvidos.
- Publicar metadata e candidatos de um repositório no mesmo snapshot atômico. Snapshots legados sem informação necessária exigem nova sincronização antes de uma resolução verificada. Preservar o snapshot anterior quando a sincronização falhar.
- Conferir identidade, versão, arquitetura, formato, tamanho e digest do artefato contra o candidato escolhido. Separar verificação da metadata de verificação do payload e aplicar a política de confiança existente; divergências bloqueiam o uso do candidato.
- Criar a aquisição limitada de entradas solicitadas pelo planejador, com publicação verificável no cache e ciclo de vida próprio. Downloads ficam fora do planejador e do lock da transação de instalação. Catálogo ausente ou vencido deve ser tratado na fronteira de sincronização/aquisição, nunca por efeitos ocultos no planejamento.
- Capturar `ID`, `ID_LIKE`, `VERSION_ID` e `VERSION_CODENAME`, além de fatos de arquitetura/libc/loader. Registrar família e suíte dos repositórios explicitamente. Gerar configuração automática somente para mapeamentos conhecidos do host; sem mapeamento, orientar configuração explícita.
- Expor divergências de release/origem ao adicionar, sincronizar e planejar repositórios. Repositórios estrangeiros declarados continuam candidatos possíveis; avisos de origem não substituem a prova de ABI. Cobrir o [caso de release divergente](../context/reports/repository-host-version-mismatch.md).

**Aceite:** os três ecossistemas alimentam o resolvedor com restrições utilizáveis e proveniência; aquisição termina dentro dos limites; bytes divergentes são rejeitados. A configuração automática não escolhe silenciosamente outra release do host.

### Etapa 4 — Integrar resolução, planejamento puro e `--dry-run`

**Dependências:** etapas 2–3. **Arquivos principais:** [resolvedor](../crates/pkg-core/src/resolver), [planejador](../crates/pkg-core/src/planner), [engine](../crates/pkg-core/src/engine.rs), [CLI](../crates/pkg-cli/src/main.rs).

- Substituir a escolha de dependências por semelhança de nomes e o filtro obrigatório de mesmo formato pela chamada real a `Resolver::resolve`. Preferência de origem/formato apenas ordena candidatos admissíveis; evidência e restrições decidem a aceitação.
- Resolver requisitos do pacote e de cada comando até a closure completa, com limites e tratamento explícito de ciclos. Uma biblioteca com ABI compatível não satisfaz automaticamente todos os requisitos semânticos de uma dependência nominal de pacote; preservar requisitos de dados, helpers e integração ou exigir receita que os classifique.
- Receber snapshots imutáveis e devolver um plano completo, uma necessidade de entradas adicionais ou uma rejeição explicada. Necessidade de artefato aciona a aquisição somente no fluxo normal. Adaptador ainda não implementado nunca vira evidência positiva provisória.
- Incluir no plano final todos os provedores, transformações, arquivos gerados, manifests, alterações de ativação, integrações permitidas e expectativas de recuperação. O staging não pode acrescentar uma receita descoberta depois; evidência nova exige novo planejamento.
- Fazer `--dry-run` abrir estado existente apenas para leitura, sem criar banco, diretórios ou lock em uma raiz vazia. Não acessar rede, sincronizar nem baixar para temporários. Artefato local ou em cache com evidência suficiente produz plano; entradas faltantes produzem prévia incompleta e status de não sucesso.
- Mostrar origem, closure, provedores por comando, requisitos satisfeitos/faltantes, adaptações e omissões opcionais justificadas. Fixar identidades dos snapshots e artefatos; a execução revalida o que pode ter mudado antes de produzir efeitos.

**Aceite:** o caminho real local/remoto utiliza o resolvedor; alternativas, transitivas, conflitos e versões têm explicações reproduzíveis. Testes com raiz vazia e entradas sem cache comprovam ausência de escritas e rede no planejamento e no `--dry-run`.

### Etapa 5 — Resolver ELF e lançar com ambiente por comando

**Dependências:** etapas 2–4 e matriz da etapa 0. **Arquivos principais:** [inspeção ELF](../crates/pkg-core/src/host/elf.rs), [runtime](../crates/pkg-core/src/runtime), [ativação](../crates/pkg-core/src/activation), [resolvedor](../crates/pkg-core/src/resolver).

- Inspecionar estaticamente `DT_NEEDED`, `SONAME`, `PT_INTERP`, `RPATH`/`RUNPATH`, arquitetura, classe, endianness, requisitos/provisão de versões de símbolos e requisitos de libc/CPU previstos na matriz. Validar também as dependências transitivas e do interpretador; existência de um arquivo `.so` não é prova suficiente.
- Produzir `ProviderEvidence` para cada ligação consumidor → arquivo. Validar a compatibilidade efetiva antes de reutilizar provedores do store ou do host, inclusive entre formatos. Falta de evidência obrigatória resulta em `Unsupported`/`NeedsHostIntegration` ou dependência não resolvida conforme o contrato.
- Construir `runtimes/<id>/lib` com aliases explícitos para os arquivos selecionados, sem expor diretórios inteiros de pacotes não selecionados. Não inventar equivalência entre SONAMEs diferentes. Dois comandos podem usar versões diferentes do mesmo SONAME; exigências incompatíveis dentro do mesmo processo devem ser rejeitadas. Compartilhar o mesmo runtime somente quando o contrato de execução completo for idêntico; as visões referenciam payloads, sem copiá-los.
- Implementar as estratégias admitidas: link direto somente para executável estático comprovadamente autocontido; runner com execução normal compatível com o host; runner com loader glibc do host explicitamente suportado; adaptador de interpretador. Não introduzir bundles genéricos de libc estrangeira.
- O runner usa argumentos estruturados e um bootstrap estático/autocontido por arquitetura suportada. O build gera esse binário com `crt-static`, grava a configuração fora de shell ao lado do launcher e registra o digest do bootstrap. Um alvo sem toolchain estática permanece explicitamente não suportado; o runner é versionado e retido enquanto houver manifests que o usem.
- Fixar a geração uma vez e usar seu manifest até a execução. Validar ambiente e fatos relevantes do host; neutralizar `LD_LIBRARY_PATH`, `LD_PRELOAD` e `LD_AUDIT` herdados antes de lançar o alvo. Definir caminhos de módulos somente conforme a receita, sem acrescentar caminhos arbitrários do ambiente. Preservar as demais variáveis salvo regra explícita do adaptador.
- Provar a busca efetiva do loader suportado: dependências com caminho, precedência e alcance distintos de RPATH/RUNPATH, `$ORIGIN`, diretórios padrão e comportamento incompatível de execução segura. Usar `--library-path` e `--argv0` apenas na estratégia que os suporta. Rejeitar casos que não possam cumprir o plano, inclusive alterações de descoberta do próprio executável ou reexecução.
- Tratar helpers e plugins como parte do contrato. Argumentos de um loader explícito não são propagados automaticamente ao loader de um filho; não compensar isso exportando todas as bibliotecas do perfil. Validar provedores do host antes da ativação e revalidar arquivos alterados no lançamento por identidade/fingerprint; registrar a limitação de corridas com alterações externas do host.

**Aceite:** fixtures ELF confiáveis executam com o provedor escolhido e preservam argumentos, `argv[0]`, status e sinais. Uma instalação nova não altera os provedores dos runtimes existentes. A inspeção de produção não usa `ldd`, não invoca o loader sobre o payload e não executa programas do pacote; somente fixtures controladas são executadas nos testes.

### Etapa 6 — Aplicar receitas explícitas e suportar interpretadores

**Dependências:** etapas 2–5. **Arquivos principais:** [relocação](../crates/pkg-core/src/domain/relocation.rs), [formatos](../crates/pkg-core/src/format), [runtime](../crates/pkg-core/src/runtime).

- Trocar normalizações amplas por receitas declarativas versionadas, com correspondência por identidade/digest ou sintaxe suportada. Inventariar recursos obrigatórios, opcionais e não suportados. Encontrar um diretório de runtime apenas inicia a inspeção; não certifica suporte.
- Validar topologia antes de seguir links; materializar configurações explicitamente mapeadas, aplicar apenas relocação textual conhecida e verificar novamente grafo, digests e requisitos de execução. Proibir substituição genérica de bytes em binários.
- Implementar materialização como `Renviron.ucf` → destino conhecido somente para a receita correspondente. `.ucf` e `.default` não autorizam copiar todos os templates. Origem ausente, destino não determinado pela receita, ambiguidade ou erro de cópia abortam; todo arquivo gerado entra no manifest e no plano.
- Relativizar links absolutos apenas quando o destino interno ou gerado estiver comprovado. Rejeitar escapes, hardlinks inválidos, ciclos e links obrigatórios pendentes. Referências ao host ou a outro pacote exigem integração/runtime explícito, sem destinos adivinhados.
- Separar defaults imutáveis no store de configuração/dados mutáveis nos locais do usuário definidos pela receita. Registrar ownership das ações e preservar edições do usuário em upgrade e remoção.
- Entregar primeiro o adaptador Python: versão e interpretador escolhidos, shebang/argumentos, módulos e extensões nativas verificados, ambiente por comando e fixture ALPM real. O adaptador genérico já cobre scripts puros Perl e Node quando os intérpretes estão disponíveis; extensões nativas e as famílias R/Ruby continuam condicionadas às suas próprias fixtures. Uma receita pontual de configuração R não declara suporte ao runtime R completo.
- Manter scripts de mantenedor desativados. Integração obrigatória desconhecida bloqueia; omissão opcional deve ser classificada pela receita e aparecer no plano/resultado.

**Aceite:** receitas determinísticas produzem exatamente os arquivos planejados; templates não relacionados permanecem intactos; falhas e links inválidos impedem ativação. Python executa pelo interpretador e módulos escolhidos sem variáveis globais; configurações do usuário sobrevivem a upgrade e remoção.

### Etapa 7 — Publicar a closure e a geração com recuperação durável

**Dependências:** etapas 1–6 e coordenação com M5. **Arquivos principais:** [engine](../crates/pkg-core/src/engine.rs), [transações](../crates/pkg-core/src/transaction), [store](../crates/pkg-core/src/store), [ativação](../crates/pkg-core/src/activation), [banco](../crates/pkg-core/src/state/db.rs), [lock](../crates/pkg-core/src/lock/process_lock.rs).

Implementar o layout da contraproposta: `cache/artifacts`, `staging/<transação>`, `store/<realization-id>`, `runtimes/<id>`, `profiles/<perfil>/generations/<id>`, `current`, `bin -> current/bin` e `state/pkg.db`.

1. Adquirir/verificar entradas fora do lock e obter plano completo. Adquirir lock de escrita, recuperar transações anteriores e revalidar snapshots, artefatos, fatos do host e ownership. Invalidar/replanejar se necessário; não prosseguir com decisões obsoletas.
2. Persistir receipt com gerações anterior/nova, objetos esperados e distinção entre criação e reutilização. Preparar staging no filesystem de destino, extrair, adaptar e gerar exatamente o previsto. Comparar a árvore final com os manifests e pós-condições.
3. Promover objetos e runtimes por operação atômica no mesmo filesystem, sem sobrescrever objetos existentes incompatíveis. Reutilização exige verificação de identidade/integridade e registro no receipt; o rollback não pode apagar objetos apenas reutilizados.
4. Preparar e tornar durável a geração completa, incluindo comandos preservados e suas referências. Resolver conflitos de ownership antes da publicação. Entrypoints e manifests devem apontar para objetos prontos e imutáveis.
5. Trocar `current` atomicamente, persistir no SQLite as referências/ownership/evidências correspondentes e concluir o journal. Especificar e testar a ordem de durabilidade de arquivos, diretórios, ponteiro e banco. Leitores usam a geração selecionada, sem depender de uma segunda leitura de `current`.

| Ponto de interrupção | Recuperação exigida |
|---|---|
| Antes da promoção | Limpar apenas staging pertencente à transação; manter a geração anterior. |
| Objetos promovidos, perfil ainda antigo | Preservar reutilizados; tratar criados conforme receipt e referências, sem apagar objetos de ownership incerto. |
| Perfil trocado, banco ainda antigo | Validar a nova geração e concluir o commit, ou restaurar a anterior e reconciliar o estado. Preservar objetos cuja utilização seja incerta. |
| Banco confirmado, journal incompleto | Confirmar o estado já publicado e finalizar a recuperação de forma idempotente. |
| Ownership inconsistente ou arquivo substituído pelo usuário | Interromper com diagnóstico; não remover nem sobrescrever arquivos desconhecidos. |

**Aceite:** fault injection em cada fronteira deixa a instalação antiga ou nova consistente após recuperação. Leitores nunca veem geração parcialmente construída; escritores concorrentes serializam. A troca governa novos lançamentos, não muda processos já em execução e não torna integrações externas atomicamente reversíveis.

### Etapa 8 — Preservar referências em upgrade, remoção e migração

**Dependências:** etapa 7. **Arquivos principais:** [engine](../crates/pkg-core/src/engine.rs), [estado](../crates/pkg-core/src/state), [ativação](../crates/pkg-core/src/activation), [recuperação](../crates/pkg-core/src/transaction/recovery.rs), [runtime](../crates/pkg-core/src/runtime).

- Persistir arestas comando/runtime → provedor e raízes de retenção: gerações ativas/retidas e transações incompletas. Atualizações criam novos objetos/manifests e revalidam consumidores afetados; não substituem um arquivo de biblioteca utilizado por outro runtime.
- Distinguir retirar comandos de um provedor de remover seu payload. Preservar o objeto enquanto referenciado; recusar remoção que invalide consumidor ativo ou produzir plano explícito para substituir/remover os dependentes. Nunca deixar referência pendente.
- Manter closures das gerações anteriores disponíveis para recuperação e futura operação de rollback. Não coletar fisicamente objetos desanexados automaticamente: um processo ainda pode carregar plugin, abrir dados ou executar helper depois. Definir leases/tempo de vida e seus testes em uma entrega específica de GC do M5.
- Migrar instalações legadas como não verificadas, sem presumir que estejam quebradas ou certificar digests ausentes. Usar artefato original verificado quando disponível; caso contrário, readquirir conforme a política ou preservar a ativação antiga com diagnóstico.
- Construir a geração de destino completa antes do corte e manter a representação anterior recuperável. Definir protocolo específico para converter o diretório legado `bin` em indireção de geração; não presumir que renomear um diretório não vazio para um symlink seja uma troca atômica comum. Se o corte seguro não for possível, falhar preservando o perfil existente.
- Detectar comandos/links substituídos pelo usuário como conflitos. Congelar dependências legadas e preservar `profile/lib` enquanto alguma ativação legada retida depender dele. Novos runtimes verificados não podem misturar seleção explícita com busca global legada.

**Aceite:** remoção de provedor mantém consumidores válidos; troca/restauração de geração mantém a closure correspondente; migração incompleta não ganha selo de verificação. Falhas preservam perfil anterior, configuração e arquivos do usuário.

### Etapa 9 — Comprovar suporte e atualizar a documentação pública

**Dependências:** etapas 1–8. **Arquivos principais:** [testes](../tests), [contrato de install](../context/commands/install.md), [referência da CLI](cli-reference.md), [README](../README.md), [guia de uso](README.md), [milestones](../context/roadmap/milestones.md), [release gates](../context/roadmap/release-gates.md).

- Construir fixtures ELF confiáveis de consumidor/provedor e empacotar em Debian, RPM e ALPM. Testar arquivos locais e repositórios controlados, incluindo cada combinação de origem consumidor/provedor anunciada. Comprovar qual biblioteca foi usada por comportamento ou marcador próprio da fixture, sem depender apenas do sucesso do processo.
- Cobrir ABI incompatível, interpretador ausente, conflitos no mesmo processo, versões independentes por comando, dependências transitivas, helpers/plugins, runtime Python e todas as falhas/recuperações das etapas anteriores. Arquivos `.so` com conteúdo fictício não satisfazem o gate de execução.
- Expor no resultado provedores exatos, origem, adaptações, omissões opcionais e causas de rejeição. Publicar matriz com host, arquitetura, libc/loader, runtime, formatos do consumidor/provedor, estratégia, resultado e referência ao teste/CI. Combinações sem prova permanecem não suportadas.
- Atualizar quickstarts e referências quando o novo caminho estiver entregue: retirar a necessidade de exportar caminhos globais de bibliotecas para comandos migrados e documentar a transição dos perfis legados. Não editar configurações de shell do usuário automaticamente.
- Atualizar comandos afetados de install/remove/upgrade e futuros comandos de geração no mesmo conjunto de mudanças de comportamento. Relatar os gates efetivamente aprovados no roadmap; não converter checklist de projeto em evidência de execução.

**Aceite:** G1–G10 têm resultados reproduzíveis, escopo declarado e links para evidência. Documentação e saída do programa prometem somente combinações comprovadas.

## 5. Ordem de integração e gates

Ordem de dependência: **0 → 2 → 3 → 4 → 5 → 6 → 7 → 8 → 9**. A etapa 1 pode ser entregue após o contrato da etapa 0, mas deve estar concluída antes da publicação da etapa 7. As etapas 2–6 podem acrescentar verificadores e receitas incrementalmente; requisitos ainda não implementados continuam bloqueados. Testes acompanham cada etapa; a etapa 9 reúne a prova do caminho completo.

Os gates abaixo correspondem à [seção 13 da contraproposta](../context/design/rootless-execution-counter-proposal.md#13-acceptance-gates). Todos permanecem pendentes; as etapas indicadas contribuem para a aprovação, não a concedem automaticamente.

| Gate | Etapas | Evidência mínima |
|---|---|---|
| G1 — Planejamento puro | 2–4, 7 | Raiz vazia sem escritas; dry-run sem rede/cache novo; entradas ausentes explícitas; revalidação; toda saída realizada presente no plano. |
| G2 — Evidência | 2–6 | ELF/arquitetura/libc/interpretador/símbolos inválidos e integração obrigatória desconhecida bloqueiam; inspeção não executa payload. |
| G3 — Execução real | 5–7, 9 | Consumidor usa provedor real pelo comando do perfil em cada combinação Debian/RPM/ALPM declarada. |
| G4 — Semântica do loader | 5, 9 | RPATH/RUNPATH, dependências com caminho, transitivas, `$ORIGIN`, ambiente e mudanças no host cumprem a estratégia ou são rejeitados. |
| G5 — Independência dos comandos | 4–5, 7–9 | Mesmo SONAME com versões distintas por comando; conflito interno rejeitado; novo pacote não muda provedor anterior; helpers/plugins cobertos. |
| G6 — Runtime e runner | 5–6, 9 | Bootstrap estático, versões/módulos/extensões, argumentos especiais, status/sinais e troca de geração comprovados. |
| G7 — Integridade da adaptação | 2, 6–8 | Mapeamento `.ucf` exato, erros de cópia, topologia, inventário, determinismo e mudanças de receita/prefixo; configuração mutável preservada. |
| G8 — Recuperação | 7–8 | Falhas nas fronteiras de promoção, troca e commit; preservação de reutilizados/arquivos do usuário; leitores sem geração parcial. |
| G9 — Retenção e migração | 7–8 | Referências preservadas em remoção/restauração, concorrência serializada e migração legada recuperável sem falsa verificação. |
| G10 — Suporte e resultados | 1, 3–4, 9 | Instalação local/remota fiel, falhas propagadas, proveniência/receitas visíveis e matriz de suporte publicada. |

Além desses gates, preservar os testes de parsing limitado, traversal, hardlinks, duplicação de caminhos, checksum, confiança de repositório e coexistência com o gerenciador nativo. O novo caminho não pode enfraquecer essas garantias.

## 6. Verificação e condição de conclusão

Para cada mudança de implementação, executar testes focados na fronteira alterada. Para a integração final, exigir:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Complementar com execução das fixtures nos hosts da matriz, provas do runner distribuído e testes de interrupção/concorrência. Uma suíte aprovada no host de desenvolvimento não certifica todas as distribuições ou arquiteturas.

O plano estará concluído quando as decisões da etapa 0 estiverem formalizadas, as entregas 1–9 implementadas e G1–G10 aprovados com evidência para o escopo publicado. Até lá, distinguir **resolvedor aprovado**, **instalação legada existente** e **novo contrato de execução ainda pendente**. Integração desktop ampla, novos runtimes e coleta física de objetos continuam sujeitos aos seus próprios gates.
