# Relatório — Runtime cross-distro a partir do caso `reflector`

Status: **relatório de análise / evidência experimental**  
Data: **2026-09-15**  
Escopo: `pkg` executando um pacote ALPM/Arch em Ubuntu 26.04.1 LTS  
Autoridade: este documento **não substitui** ADRs, invariantes, requirements ou design canônico.

---

## 1. Resumo executivo

O teste com `reflector` provou que o `pkg` já consegue atravessar uma parte relevante da fronteira entre ecossistemas:

1. resolver um pacote em repositório Arch (`arch-extra`);
2. baixar um artefato `.pkg.tar.zst`;
3. extrair o payload completo para o store isolado do `pkg`;
4. preservar arquivos fora de `usr/bin`, incluindo módulos Python e configuração XDG;
5. ativar o executável no profile por symlink;
6. executar o pacote no Ubuntu **quando o ambiente de runtime correto é fornecido**.

O mesmo teste também demonstrou que **extração bem-sucedida não significa execução compatível**. O primeiro `reflector --help` falhou porque o script contém um shebang absoluto `#!/usr/bin/python`, caminho inexistente no host. Depois que `/usr/bin/python` passou a existir no host, a execução avançou, mas falhou em `ModuleNotFoundError: No module named 'Reflector'`. O módulo existia corretamente no store do `pkg`, porém o Python do host não incluía o `site-packages` daquele store em `sys.path`. Ao injetar o caminho correto via `PYTHONPATH`, `reflector --help` funcionou.

A conclusão arquitetural é direta:

> O próximo problema não é mais “como extrair um pacote Arch no Ubuntu”. É “como construir, verificar e ativar uma **closure de runtime relocável** sem transformar o host em Arch, sem escrever payloads em `/usr` e sem contaminar globalmente o shell do usuário”.

O caso também mostrou que `pkg install python-is-python3` não pode ser considerado equivalente a `apt install python-is-python3`. No modelo rootless do `pkg`, o payload foi instalado no store; já o pacote Debian nativo tem valor justamente porque produz um caminho absoluto do host (`/usr/bin/python -> python3`). Reproduzir esse efeito silenciosamente violaria as invariantes do projeto. Portanto, o resultado correto para esse tipo de dependência não é “instalar globalmente por trás do usuário”, mas classificá-la como requisito de runtime que precisa ser satisfeito por uma adaptação segura, por uma capability do host ou, quando inevitável, por uma integração explícita.

A recomendação central deste relatório é uma arquitetura híbrida:

- **store imutável e isolado** permanece como está;
- **profile** continua sendo a superfície de ativação;
- cada comando ativado recebe uma **runtime closure** calculada;
- wrappers/launchers aplicam ambiente **por processo**, não globalmente;
- shebangs e ELF são adaptados apenas por regras determinísticas e verificáveis;
- capabilities descrevem requisitos reais (`python >= 3`, SONAMEs, loader, caminhos de runtime), sem inferir compatibilidade apenas por nome de pacote;
- casos que exigem um FHS real de outra distro devem usar futuramente uma camada isolada (por exemplo, `bubblewrap`) ou ser recusados;
- host integration continua explícita, tipada e reversível.

Isso mantém o `pkg` alinhado ao objetivo canônico do projeto: tratar `.deb`, `.rpm` e `.pkg.tar.zst` como **artefatos de entrada**, não como licença para reproduzir implicitamente toda a semântica do package manager de origem.

---

## 2. Relação com a arquitetura atual

O contexto canônico já define vários princípios que o teste reforça:

- payloads não devem ser espalhados diretamente em `/usr`;
- databases de `dpkg`, RPM e libalpm não devem ser alteradas implicitamente;
- dependency names entre Debian, RPM e ALPM não são considerados equivalentes;
- compatibilidade é expressa por capabilities e evidência do host;
- extração bem-sucedida não equivale a instalação bem-sucedida;
- operação rootless é o padrão;
- host integration deve ser estreita, explícita e reversível.

O design de cross-distro também já divide adaptação em níveis:

- **Level 0:** sem adaptação;
- **Level 1:** apenas ativação de paths;
- **Level 2:** relocation determinística / wrapper environment;
- **Level 3:** host integration explícita;
- **Level 4:** runtime da distro de origem necessário; container futuro ou rejeição.

O `reflector` é um caso concreto de **Level 2** para `--help`: o payload é utilizável desde o store, desde que o launcher forneça o interpretador e o `site-packages` corretos. Entretanto, certas operações do programa continuam semanticamente Arch-specific (mirrorlist e infraestrutura de mirrors do Arch), então “o processo executa” não deve ser confundido com “todas as funções do pacote são apropriadas ao host”.

O M3 já prevê ALPM, normalized constraint IR, capabilities, provider selection e corpus de compatibilidade. O principal gap evidenciado pelo experimento é que o M3 precisa também de um modelo explícito para **runtime closure e relocation evidence**.

---

## 3. Reprodução do experimento

### 3.1 Instalação do pacote Arch

Comando:

```text
pkg install reflector
```

Resultado observado:

```text
Resolving package target 'reflector'...
Found reflector 2023-5 [alpm] in arch-extra
Downloading reflector...
Installed reflector 2023-5
Activated binaries in profile 'default':
  - reflector
```

O binário ativado ficou em:

```text
~/.local/share/pkg/profiles/default/bin/reflector
```

resolvendo para:

```text
~/.local/share/pkg/store/447965cc806b-reflector-2023-5/usr/bin/reflector
```

O store continha corretamente:

```text
.../usr/bin/reflector
.../usr/lib/python3.14/site-packages/Reflector.py
.../etc/xdg/reflector
```

Portanto, o extractor/store **não descartou** o payload auxiliar.

### 3.2 Falha 1 — shebang absoluto

O script começa com:

```text
#!/usr/bin/python
```

No host Ubuntu havia:

```text
/usr/bin/python3 -> python3.14
/usr/bin/python3.14
```

mas não `/usr/bin/python`.

Resultado:

```text
The file specified the interpreter '/usr/bin/python',
which is not an executable command.
```

Esse comportamento é esperado pelo kernel: em um script executável, o caminho do shebang é resolvido como um **pathname absoluto**. O `PATH` do profile não participa da resolução de `#!/usr/bin/python`.

### 3.3 `pkg install python-is-python3` não altera o host

O `pkg` encontrou o pacote em repositórios Debian/Ubuntu e instalou seu payload no ambiente controlado pelo `pkg`. Depois disso:

```text
which python
# nenhum resultado

ls -l /usr/bin/python
# No such file or directory
```

Isso não é, por si só, falha do store. É consequência direta do modelo rootless: instalar no store **não deve** reproduzir silenciosamente os efeitos absolutos que o pacote teria quando instalado nativamente em `/`.

Ao instalar o pacote com APT no host:

```text
sudo apt install python-is-python3
```

passou a existir:

```text
/usr/bin/python -> python3
```

A execução então avançou para a próxima falha.

### 3.4 Falha 2 — módulo Python fora do `sys.path`

Erro:

```text
ModuleNotFoundError: No module named 'Reflector'
```

O módulo estava no store:

```text
~/.local/share/pkg/store/447965cc806b-reflector-2023-5/usr/lib/python3.14/site-packages/Reflector.py
```

mas o Python do host não adiciona stores arbitrários do `pkg` ao seu `sys.path`.

A execução funcionou com:

```text
env PYTHONPATH="$HOME/.local/share/pkg/store/447965cc806b-reflector-2023-5/usr/lib/python3.14/site-packages" reflector --help
```

O help completo foi exibido.

### 3.5 Resultado factual

O teste permite afirmar:

| Propriedade | Resultado |
|---|---|
| Resolver pacote ALPM | OK |
| Baixar `.pkg.tar.zst` | OK |
| Extrair payload completo | OK |
| Store isolado | OK |
| Ativar executável | OK |
| Resolver shebang absoluto | Não |
| Construir `PYTHONPATH` da closure | Não |
| Executar com ambiente manual correto | OK |
| Provar compatibilidade funcional completa com Ubuntu | Não |

---

## 4. Quatro conceitos que não podem ser tratados como sinônimos

### 4.1 Installability

Pergunta:

> O artefato pode ser parseado, verificado, extraído e materializado no store sem violar invariantes?

Para `reflector`: **sim**.

### 4.2 Dependency satisfiability

Pergunta:

> Todos os requisitos necessários ao pacote podem ser satisfeitos por sua closure ou por capabilities verificadas do host?

Para `reflector`: inicialmente **não**. O interpretador esperado pelo shebang não existia e o módulo Python não estava no path de runtime.

### 4.3 Relocatability

Pergunta:

> Um pacote construído assumindo caminhos como `/usr/bin`, `/usr/lib` e `/etc/xdg` continua funcional quando seu payload reside em `~/.local/share/pkg/store/...`?

Para `reflector`: **sim, com adaptação determinística parcial**. `PYTHONPATH` resolveu o import; o shebang ainda exige adaptação ou uma capability do host.

### 4.4 Runtime/platform compatibility

Pergunta:

> Mesmo que o processo inicie, seu ABI, filesystem contract, serviços, configuração, hardware e semântica são compatíveis com o host?

Para `reflector`: `--help` é compatível. Isso **não prova** que todas as ações do programa façam sentido em Ubuntu, pois a aplicação existe para consultar e gerar configuração relacionada aos mirrors do Arch Linux.

Essas quatro dimensões devem aparecer separadamente no modelo e nos diagnósticos do `pkg`.

---

## 5. Por que package-name mapping não resolve o problema

Uma regra do tipo:

```text
Arch python -> Ubuntu python-is-python3
```

é insuficiente e pode estar errada em vários contextos.

`python-is-python3` não representa simplesmente “Python está disponível”. Sua utilidade principal é satisfazer um contrato de path absoluto (`/usr/bin/python`). Quando instalado nativamente pelo APT, ele pode criar esse path no filesystem global. Quando extraído para um store rootless, a mesma semântica não existe.

O modelo deve distinguir:

```text
source package name
    ↓
normalized requirements
    ↓
capabilities/evidence
    ↓
adaptation plan
```

Exemplos de capabilities úteis:

```text
runtime.python.major = 3
runtime.python.version >= 3.14
executable.python3
elf.soname.libssl.so.3
elf.loader = /lib64/ld-linux-x86-64.so.2
kernel.linux
arch.x86_64
filesystem.fhs-path:/usr/bin/python  # expectativa de path, não pacote
```

Uma dependência pode ser satisfeita por:

- outro pacote da closure;
- uma capability observada no host;
- relocation determinística;
- wrapper/launcher;
- host integration explícita;
- futuramente, FHS virtualizado;
- ou ser classificada como insatisfeita.

O nome do pacote de origem deve continuar preservado para diagnóstico, mas não deve ser a prova de compatibilidade.

---

## 6. Shebangs absolutos

### 6.1 O problema

Em scripts executáveis:

```text
#!/usr/bin/python
```

não significa “procure `python` no PATH”. Significa “execute exatamente `/usr/bin/python`”.

Por isso, colocar `python` em:

```text
~/.local/share/pkg/profiles/default/bin/python
```

não satisfaz esse shebang.

### 6.2 Estratégias possíveis

#### A. Exigir o path do host

Vantagem: máxima fidelidade ao pacote original.  
Problema: reduz portabilidade e pode exigir root/host integration.

Deve ser classificado como `SatisfiedByHost` quando já existe ou `NeedsHostIntegration` quando depender de mutação externa.

#### B. Reescrever o shebang

Exemplo conceitual:

```text
#!/usr/bin/python
```

para um interpretador determinado pelo plano de runtime.

Vantagem: eficiente e simples em scripts de texto.  
Risco: mudar o interpretador pode alterar semântica; `/usr/bin/env python` é ainda ambíguo porque depende do PATH.

Só deve ocorrer quando o planner tem evidência suficiente para provar o runtime esperado.

#### C. Wrapper por comando

O profile pode expor um launcher que executa explicitamente:

```text
<resolved-python> <store>/usr/bin/reflector "$@"
```

Vantagens:

- não altera o payload original;
- reversível;
- permite anexar `PYTHONPATH` da mesma closure;
- fácil de registrar no estado;
- aplica ambiente apenas ao processo alvo.

Para o MVP/M3, essa é a estratégia mais coerente com as invariantes existentes.

#### D. FHS virtualizado

Executar o programa dentro de um namespace onde `/usr/bin/python`, `/usr/lib/...` etc. são montados a partir da closure do `pkg`.

Ferramentas como `bubblewrap` podem oferecer esse modelo futuramente.

Vantagem: alta fidelidade para pacotes fortemente presos ao FHS.  
Custo: complexidade, sandboxing, mount namespaces, políticas de acesso e debugging.

Esse caminho faz mais sentido para **Level 4** do que como solução padrão.

---

## 7. Python: `sys.path`, `site-packages` e closure

O Python calcula seu `sys.path` a partir de fatores como:

- localização/versão do interpretador;
- biblioteca padrão;
- `site-packages` conhecidos pelo interpretador;
- virtual environments;
- `PYTHONPATH`;
- arquivos `.pth` e mecanismos do módulo `site`.

O fato de um módulo existir no store do `pkg` não faz com que o Python do host o descubra.

No caso observado:

```text
store/.../usr/lib/python3.14/site-packages/Reflector.py
```

foi corretamente encontrado quando esse diretório entrou no `PYTHONPATH`.

### Recomendação

Não exportar todos os `site-packages` instalados pelo `pkg` globalmente no shell.

Isso criaria problemas de:

- colisão entre versões;
- shadowing de módulos do sistema;
- import acidental entre packages sem dependência declarada;
- comportamento dependente da ordem de instalação;
- debugging difícil;
- risco de um pacote contaminar outros executáveis Python.

Preferir **PYTHONPATH por runtime closure**:

```text
activation(reflector)
  -> python capability
  -> reflector site-packages
  -> optional dependency site-packages
```

O wrapper do `reflector` recebe apenas os paths necessários à sua closure.

---

## 8. Shared libraries, loader, RPATH e RUNPATH

Pacotes nativos introduzem o mesmo problema em nível ELF.

Um executável pode depender de:

- `PT_INTERP` / dynamic linker;
- `DT_NEEDED` / SONAMEs;
- `DT_RPATH`;
- `DT_RUNPATH`;
- symbol versions;
- glibc/musl e ABI específicos;
- paths absolutos internos.

### Ordem recomendada de abordagem

1. **Inspecionar, nunca executar**, binários não confiáveis para descobrir dependências.
2. Verificar se SONAMEs necessários são satisfeitos pela closure ou pelo host.
3. Quando a closure contém as libs corretas, preferir uma estratégia determinística e scoped:
   - RPATH/RUNPATH adequado; ou
   - launcher com library path apenas para o processo.
4. Não colocar indiscriminadamente todos os `usr/lib` do store em um `LD_LIBRARY_PATH` global.
5. Rejeitar ABI incompatível mesmo quando nomes de pacotes “parecem” equivalentes.

### `LD_LIBRARY_PATH`

É útil como ferramenta de adaptação, mas ruim como política global. Pode alterar resolução de libraries de qualquer processo iniciado no shell.

Se usado, deve ser **wrapper-scoped**.

### RPATH/RUNPATH

Patch de ELF pode ser mais determinístico, especialmente porque paths do store do `pkg` são estáveis enquanto o objeto estiver instalado. Porém, patching modifica o payload executável derivado do artefato e precisa ser registrado como transformação reproduzível, com hash/evidência da origem e do resultado.

Isso merece ADR próprio antes de virar comportamento geral.

---

## 9. XDG, `pkg-config`, manpages e outros runtime paths

### `XDG_DATA_DIRS`

Pacotes podem colocar em `usr/share`:

- desktop entries;
- icons;
- schemas;
- MIME data;
- locale/data files.

Adicionar todos os stores globalmente a `XDG_DATA_DIRS` pode afetar aplicações não relacionadas. Para aplicativos desktop, o projeto já prevê host integration tipada no M4. Para dados privados do próprio aplicativo, wrapper-scoped lookup é preferível quando possível.

### `XDG_CONFIG_DIRS`

O `reflector` contém `etc/xdg/reflector`. Esse diretório pode precisar entrar no search path do processo. Novamente, o ambiente por closure evita expor configs de todos os pacotes para todo o desktop.

### `PKG_CONFIG_PATH`

Relevante principalmente para pacotes de desenvolvimento. Não deve ser prioridade do runtime MVP, mas o modelo deve permitir adicionar `usr/lib/pkgconfig` e `usr/share/pkgconfig` quando um ambiente de build/dev for explicitamente ativado.

### `MANPATH`

Pode ser agregado no profile com risco relativamente baixo, mas deve continuar separado da closure de execução. É uma integração de descoberta/documentação, não requisito para o processo funcionar.

### Outros exemplos

- `GI_TYPELIB_PATH`;
- `GSETTINGS_SCHEMA_DIR`;
- `QT_PLUGIN_PATH`;
- `QML2_IMPORT_PATH`;
- `GST_PLUGIN_PATH`;
- Java classpath;
- Perl/PHP/Ruby module paths;
- Tcl/Tk library paths;
- locale paths.

É inviável resolver todo o ecossistema com uma lista global fixa de env vars. O design precisa de **runtime adapters** extensíveis, acionados por evidência do payload e metadata.

---

## 10. Arquitetura proposta

### 10.1 Separar store, closure e activation

```text
Artifact
   |
   v
NormalizedPackage
   |
   v
Compatibility Planner
   |
   +--> HostFacts
   +--> dependency/capability IR
   +--> payload inspection
   |
   v
RuntimeClosure
   |
   +--> interpreter requirements
   +--> ELF requirements
   +--> module/search paths
   +--> runtime data paths
   +--> adaptation actions
   |
   v
Activation
   |
   +--> profile command launcher
   +--> typed host integrations (optional)
```

### 10.2 `RuntimeClosure`

Estrutura conceitual:

```rust
struct RuntimeClosure {
    packages: Vec<StoreObjectId>,
    capabilities: Vec<ResolvedCapability>,
    environment: Vec<ScopedEnvMutation>,
    interpreter: Option<InterpreterPlan>,
    elf: Option<ElfRuntimePlan>,
    integrations: Vec<PlannedIntegration>,
    compatibility: CompatibilityResult,
}
```

Não precisa virar API pública; é um modelo de domínio interno para tornar explícito o que hoje está implícito.

### 10.3 Activation por launcher

Em vez de:

```text
profile/bin/reflector -> store/.../usr/bin/reflector
```

para pacotes Level 2, usar conceitualmente:

```text
profile/bin/reflector -> pkg launcher metadata
```

O launcher resolve uma activation ID e executa:

```text
environment da closure
+ interpretador correto
+ executable/script real no store
```

Não é necessário gerar shell script para todo comando. Uma alternativa mais robusta é um único launcher do próprio `pkg`:

```text
profile/bin/reflector -> pkg-runtime-launcher
```

com dispatch pelo nome do symlink ou por metadata persistida.

Isso evita dependência de Bash/Fish/Zsh e reduz problemas de quoting.

### 10.4 Environment por processo

Regra recomendada:

> O profile global deve conter o mínimo necessário para descoberta de comandos. Variáveis que mudam resolução de runtime devem ser aplicadas no processo do comando, a partir de sua closure.

Assim:

```text
PATH do usuário
  -> profile/bin

reflector launcher
  -> PYTHONPATH específico do reflector
  -> XDG_CONFIG_DIRS necessário ao reflector
  -> interpreter selecionado
```

Não:

```text
.bashrc / config.fish
  -> PYTHONPATH de todos os packages
  -> LD_LIBRARY_PATH de todos os packages
  -> XDG_CONFIG_DIRS de todos os packages
```

---

## 11. Como tratar `python-is-python3`

O teste mostrou uma armadilha importante para o UX e para o resolver.

Quando instalado pelo `pkg`, `python-is-python3` pode ter seu payload materializado no store, mas isso **não significa** que sua finalidade de sistema foi realizada.

O planner deveria detectar que o pacote depende de um contrato de path absoluto no host e produzir algo equivalente a:

```text
Package payload: stored
Required capability: interpreter path /usr/bin/python
Status: not satisfied by pkg store
Possible adaptations:
  - wrapper with verified python3 provider
  - shebang rewrite
  - host capability already present
  - explicit host integration
```

Em outras palavras, `pkg install python-is-python3` não deve induzir o usuário a pensar que `/usr/bin/python` foi instalado globalmente.

Uma melhoria futura de diagnóstico poderia ser:

```text
Installed python-is-python3 payload into pkg store.
This package normally provides an absolute host path: /usr/bin/python.
The pkg rootless store does not modify /usr.
Capability remains unsatisfied for packages requiring that exact host path.
```

Isso transforma uma surpresa em comportamento explicável.

---

## 12. Comparação com outros sistemas

### Nix

Lições úteis:

- store imutável;
- closures explícitas;
- profiles como superfície de ativação;
- fixups/patching e referências ao store.

Limite da comparação: Nix normalmente constrói pacotes já adaptados ao `/nix/store`; ele não promete que qualquer binário arbitrário de outra distro será relocável sem fixup.

### GNU Guix

Semelhante ao Nix em store, profiles e ambientes. Demonstra o valor de separar conteúdo instalado de ambiente ativado.

### Homebrew / Linuxbrew

Bottles e kegs vivem em prefixos próprios. Homebrew executa fixups e trabalha com packages construídos para seu modelo de prefixo. A lição é importante: **relocabilidade é uma propriedade que precisa ser construída ou verificada**, não presumida.

### Conda

Conda environments mostram um modelo forte para runtime closure por ambiente: prefixos isolados, scripts de activation e packages que conhecem o prefixo do environment. Também usa mecanismos de prefix replacement. A lição é separar ambientes para evitar contaminação cruzada.

### Spack

Spack trabalha bem com múltiplas versões e dependências em prefixes separados, normalmente utilizando RPATH para apontar para a dependency closure. É uma referência melhor que um `LD_LIBRARY_PATH` global para software nativo complexo.

### Flatpak

Flatpak resolve grande parte do problema por runtime + sandbox + filesystem contract previsível. Para `pkg`, isso é uma referência para o futuro Level 4, não para o caminho padrão de CLI packages.

### AppImage

AppImage monta um AppDir/bundle e executa a aplicação com seus recursos próximos. Demonstra outra estratégia: aproximar filesystem e dependências da aplicação em vez de tentar misturá-las ao host.

### pacman/ALPM e dpkg/APT

Os managers nativos podem assumir paths canônicos e executar lifecycle scripts porque controlam o filesystem e database da própria distro. O `pkg` deliberadamente não possui essa licença. Portanto, simplesmente “extrair como pacman/dpkg extrairia” nunca será suficiente para packages dependentes de semântica de sistema.

---

## 13. Wrappers vs profile environment vs patching vs sandbox

| Estratégia | Vantagem | Risco/custo | Uso recomendado |
|---|---|---|---|
| Symlink direto | mínimo overhead | só funciona em Level 0/1 | pacotes autocontidos |
| Env global do profile | simples | contamina packages/processos | apenas paths de descoberta de baixo risco |
| Wrapper/launcher por comando | scoped, reversível | planner/metadata mais complexo | padrão para Level 2 |
| Shebang patch | rápido e determinístico | pode alterar semântica | quando interpretador é provado |
| ELF RPATH/RUNPATH patch | closure nativa previsível | transformação binária complexa | packages ELF relocáveis |
| `LD_LIBRARY_PATH` no wrapper | fácil | precedence/ABI hazards | fallback scoped |
| FHS virtual via namespace | alta compatibilidade | complexidade operacional | Level 4/fallback |
| Escrever em `/usr` | fidelidade à distro origem | viola modelo rootless/coexistência | não usar implicitamente |

---

## 14. Riscos reais de um “universal package manager”

O projeto precisa manter limites explícitos. Alguns pacotes **não devem** ser adaptados automaticamente.

### 14.1 libc e dynamic loader

Misturar glibc/musl, loaders ou versões com symbol requirements incompatíveis pode causar crash ou comportamento indefinido. Nome do pacote não resolve isso.

### 14.2 Kernel, DKMS e drivers

Exigem integração profunda com kernel/toolchain/boot. Permanecem fora do escopo normal.

### 14.3 init/services

Pacotes que assumem systemd units, users/groups, tmpfiles, sysusers ou scripts de lifecycle precisam de integração explícita e policy própria.

### 14.4 PAM, NSS, bootloader, package database

Devem continuar default-deny.

### 14.5 Maintainer scripts

Muitos packages Debian/RPM dependem de scripts para ficar funcional. Executá-los cegamente fora da distro de origem destrói a propriedade de segurança do modelo. Inventariar não é executar.

### 14.6 Packages que escrevem em paths absolutos em runtime

Mesmo que o executable seja relocável, o programa pode tentar escrever em `/var`, `/etc`, `/usr/share` ou sockets esperados. Esse comportamento precisa de sandbox/redirect ou rejeição.

### 14.7 Semântica específica da distro

`reflector` é o exemplo perfeito: executar o programa é possível; isso não transforma Ubuntu em Arch nem torna todas as operações semanticamente úteis.

O posicionamento defensável é:

> `pkg` pode universalizar **ingestão, normalização, store, resolução e adaptação de classes suportadas de aplicações**. Ele não pode prometer que qualquer pacote de qualquer distro será semanticamente portátil.

---

## 15. Mudanças recomendadas no modelo de compatibilidade

### 15.1 Adicionar dimensão de relocation

Hoje os resultados de requirement podem continuar existindo, mas a install plan deveria agregar uma avaliação do pacote:

```text
Installability: Supported | Unsupported
Dependencies: Satisfied | Partial | Unsatisfied | Unknown
Relocation: None | PathActivation | Deterministic | NeedsFhsRuntime | Unsupported
RuntimeCompatibility: Verified | Partial | Unknown | Incompatible
```

### 15.2 Registrar evidência

Cada adaptação deve guardar:

- arquivo afetado;
- tipo da adaptação;
- razão;
- capability que a justificou;
- hash original;
- hash/materialização resultante quando houver patch;
- packages da closure usados;
- host facts relevantes.

Isso deixa `pkg doctor`, uninstall, upgrade e debugging possíveis.

### 15.3 Não tratar `Unknown` como sucesso

Se um programa depende de path/runtime que o planner não entende, o resultado correto é `Unknown` ou `NeedsIntegration`, não “Installed successfully” sem ressalvas.

---

## 16. Proposta de roadmap técnico

### Fase A — fechar o caso `reflector`

Objetivo: provar runtime closure Python sem mutar `/usr`.

- detectar shebangs de scripts ativados;
- identificar `/usr/bin/python` como expectativa não satisfeita;
- detectar `usr/lib/python*/site-packages` do package/closure;
- gerar launcher scoped;
- selecionar um `python3` compatível via capability do host ou closure;
- anexar `PYTHONPATH` ao processo;
- executar `reflector --help` sem `env` manual e sem `apt install python-is-python3` como requisito do `pkg`.

Critério de aceite:

```text
pkg install reflector
reflector --help
```

funciona em um host de teste suportado, e o install plan explica todas as adaptações.

### Fase B — closure/runtime metadata

- introduzir modelo interno `RuntimeClosure` ou equivalente;
- registrar env mutations scoped;
- distinguir host capability de package capability;
- persistir activation metadata;
- garantir uninstall idempotente.

### Fase C — ELF runtime

- inspecionar `PT_INTERP`, `DT_NEEDED`, RPATH/RUNPATH e symbol versions quando viável;
- construir closure de SONAMEs;
- experimentar wrapper-scoped library path;
- avaliar patch determinístico de RUNPATH/RPATH;
- criar ADR antes de escolher política geral de ELF patching.

### Fase D — runtime adapters

Adapters iniciais:

- Python;
- ELF/shared libraries;
- XDG data/config de processo;
- manpages/documentation activation.

Node, Java, Ruby, Perl, Qt/GI etc. só entram quando fixtures reais justificarem.

### Fase E — Level 4 / FHS compatibility

Somente depois de Level 2 estar sólido:

- avaliar `bubblewrap`/mount namespaces;
- montar closure em filesystem virtual previsível;
- bloquear acesso perigoso por default;
- comparar custo operacional contra simples rejeição.

---

## 17. Casos de teste recomendados

### Python

1. `reflector` — shebang absoluto + módulo em `site-packages`.
2. script com `#!/usr/bin/env python3`.
3. package com dependency Python adicional na closure.
4. duas aplicações exigindo versões/module paths conflitantes; uma não pode contaminar a outra.

### ELF

1. binário estático — Level 0.
2. binário usando apenas libs do host — `SatisfiedByHost`.
3. binário usando `.so` dentro do próprio package — Level 2.
4. SONAME presente, mas symbol version incompatível — deve falhar.
5. interpreter/loader incompatível — deve falhar ou exigir Level 4.

### Paths/config

1. package com `etc/xdg` próprio.
2. package com `usr/share` data requerida em runtime.
3. package que tenta escrever em path absoluto não permitido.

### Semântica de distro

1. `reflector`: execução técnica funciona, mas capability/diagnóstico mantém indicação de semântica Arch-specific.
2. package systemd-only em host sem systemd — `NeedsIntegration`/`Unsupported`.
3. package com maintainer script obrigatório — não fingir sucesso.

### Segurança

1. shebang apontando para path inesperado/controlado pelo pacote;
2. env path injection;
3. symlink escape em payload;
4. library precedence attack via wrapper;
5. package tentando usar loader/library fora da closure planejada.

---

## 18. Decisões que merecem ADR

O experimento não justifica mudar imediatamente o design canônico. Ele justifica abrir decisões específicas:

### ADR candidata A — Runtime activation model

Decidir entre:

- symlink direto para Level 0/1;
- launcher central para Level 2;
- formato e ownership da activation metadata.

### ADR candidata B — Script interpreter adaptation

Definir:

- quando shebang pode ser reescrito;
- quando deve ser wrapper;
- como interpreter capability é provada;
- política para `/usr/bin/env`.

### ADR candidata C — ELF relocation policy

Definir prioridade entre:

- host SONAMEs;
- closure libs;
- RUNPATH/RPATH patch;
- scoped library path;
- rejeição.

### ADR candidata D — FHS compatibility fallback

Definir se Level 4 usa sandbox/mount namespace ou apenas rejeição no v1.

---

## 19. Recomendação final

O teste não indica que o `pkg` precise abandonar o store isolado. Indica o contrário: **o store está funcionando**, e foi justamente ele que permitiu localizar com precisão o payload necessário para executar `reflector`.

O ponto fraco atual é a ativação: um symlink em `profile/bin` modela apenas packages simples. O próximo salto arquitetural deve transformar activation em uma operação consciente da runtime closure.

A direção recomendada é:

```text
artifact
  -> normalized package
  -> compatibility/capability planning
  -> isolated store
  -> runtime closure
  -> command-scoped launcher
  -> optional typed host integration
```

Para o caso `reflector`, o comportamento alvo deveria ser:

```text
pkg install reflector
  -> store payload
  -> detect Python script
  -> resolve Python capability
  -> discover reflector site-packages
  -> create scoped activation
  -> no write to /usr
  -> no global PYTHONPATH

reflector --help
  -> launcher
  -> scoped PYTHONPATH
  -> verified Python interpreter
  -> store/usr/bin/reflector
  -> success
```

Esse desenho preserva as invariantes atuais, evita transformar `.bashrc`/`config.fish` em um agregado de paths, não depende de hardcode por package name e cria uma base extensível para Python, ELF e outros runtimes.

A ambição “cross-distro” continua tecnicamente viável **desde que o produto assuma limites explícitos**: o `pkg` deve universalizar o que consegue provar e adaptar deterministicamente; o que exigir semântica de sistema da distro de origem deve ser classificado, isolado ou recusado — nunca tratado como sucesso por conveniência.

---

## 20. Referências

### Contexto interno do projeto

- `context/AGENTS.md`
- `context/README.md`
- `context/design/compatibility-model.md`
- `context/design/cross-distro-installation.md`
- `context/roadmap/milestones.md`
- `context/adr/ADR-010-profile-based-binary-activation.md`
- `context/adr/ADR-016-capability-based-cross-distro-compatibility.md`
- `context/adr/ADR-018-explicit-host-integration.md`

### Fontes externas

- Arch Linux package file list — Reflector: https://archlinux.org/packages/extra/any/reflector/files/
- Debian `python-is-python3`: https://packages.debian.org/sid/python-is-python3
- Python documentation — `sys.path`: https://docs.python.org/3/library/sys_path_init.html
- Python documentation — `site`: https://docs.python.org/3/library/site.html
- XDG Base Directory Specification: https://specifications.freedesktop.org/basedir/latest/
- ELF dynamic linker (`ld.so`): https://man7.org/linux/man-pages/man8/ld.so.8.html
- Nix profiles: https://nix.dev/manual/nix/latest/command-ref/new-cli/nix3-profile
- GNU Guix profiles: https://guix.gnu.org/manual/en/html_node/Invoking-guix-package.html
- Homebrew bottles: https://docs.brew.sh/Bottles
- Conda environments: https://docs.conda.io/projects/conda/en/latest/user-guide/concepts/environments.html
- Spack RPATH packaging model: https://spack.readthedocs.io/
- Flatpak sandbox permissions: https://docs.flatpak.org/en/latest/sandbox-permissions.html
- AppImage runtime concepts: https://docs.appimage.org/
