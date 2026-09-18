# Relatório: reutilização de bibliotecas entre `.deb`, `.rpm` e ALPM

Data: 2026-09-17  
Status: comportamento implementado para o caminho verificado descrito na matriz; a cobertura de hosts, loaders e runtimes continua limitada aos gates publicados.

## Resposta operacional

Sim. Quando `test.deb` realmente fornece uma biblioteca ELF e `test2.rpm` exige o mesmo SONAME, o `pkg` pode reutilizar o objeto já instalado. A decisão não depende de o pacote ser Debian ou RPM: ela usa a capacidade compartilhada, a arquitetura, o SONAME, as versões de símbolos, o loader e as dependências transitivas.

O fluxo atual funciona assim:

1. O adaptador do formato normaliza dependências, capacidades e `Provides` versionado. Debian, RPM-MD e ALPM preservam a semântica de versão da sua origem.
2. O planejador/resolvedor escolhe a closure de pacotes e capacidades. A CLI adquire os artefatos selecionados e replaneja quando necessário.
3. Durante o staging, a inspeção ELF verifica `DT_NEEDED`, `SONAME`, `PT_INTERP`, `RUNPATH`/`RPATH`, arquitetura, classe, endianness e versões de símbolos.
4. O runtime grava uma view por comando com aliases apenas para os provedores escolhidos. O runner usa essa view; não depende de um `LD_LIBRARY_PATH` global do perfil.
5. A coleta física de um provedor é impedida enquanto uma geração ou runtime ainda referencia o objeto. O pacote pode sair do perfil, mas consumidores continuam executando com a closure retida.

## Exemplos

| Situação | Resultado |
|---|---|
| Um `.deb` fornece `libtest.so.1` e um `.rpm` exige esse SONAME no mesmo perfil | O provedor Debian pode ser reutilizado, desde que a evidência ELF seja compatível. |
| O `.deb` apenas declara dependência de `libtest.so.1`, sem conter ou fornecer o arquivo | A declaração não transforma o pacote em provedor; outro candidato precisa satisfazer a capacidade. |
| O consumidor exige `libtestrpm.so.1`, mas só existe `libtest.so.1` | Nomes/SONAMEs diferentes não são equivalentes automaticamente. |
| O provedor tem arquitetura, loader ou símbolos incompatíveis | A instalação verificada é rejeitada antes da publicação. |
| O provedor está ausente | A resolução solicita um artefato admissível; a aquisição não escolhe outro pacote apenas por semelhança de nome. |
| O pacote é ALPM (`.pkg.tar.zst`) | O mesmo contrato é aplicado; ALPM é o ecossistema, e o detector trata o nome real do arquivo. |

## Limites atuais

- A compatibilidade é uma decisão baseada em evidências, não uma prova de que toda a API ou comportamento da biblioteca é semanticamente igual entre distribuições.
- A matriz executada neste checkout cobre host x86_64/glibc, runner nativo, Python, scripts puros Perl/Node e a fixture ELF cross-format Debian → RPM/ALPM. Outras arquiteturas, musl, loaders estrangeiros, R/Ruby e extensões nativas de interpretadores permanecem fora do suporte verificado.
- A ativação legada ainda mantém links em `profiles/<perfil>/lib` para compatibilidade com instalações antigas. Runtimes novos usam views por comando.
- Scripts de mantenedor, serviços privilegiados e alterações nos bancos de `apt`, `dnf` ou `pacman` continuam bloqueados pela política rootless.

## Evidência no código e nos testes

- [`NormalizedPackage` e `RemotePackage`](crates/pkg-core/src/domain/package.rs) carregam constraints, capabilities e `versioned_provides`.
- Os adaptadores [`deb`](crates/pkg-core/src/repository/deb.rs), [`rpm_md`](crates/pkg-core/src/repository/rpm_md.rs) e [`alpm_sync`](crates/pkg-core/src/repository/alpm_sync.rs) preservam esses campos no catálogo remoto.
- [`Resolver`](crates/pkg-core/src/resolver/mod.rs) resolve por capacidade e restrição, sem equivalência nominal entre distros.
- [`inspect_elf_with_extra_paths`](crates/pkg-core/src/host/elf.rs) resolve a closure ELF e rejeita caminhos que escapem do staging.
- [`runtime`](crates/pkg-core/src/runtime/mod.rs) grava evidência dos provedores e verifica novamente ABI, digest e símbolos na execução.
- [`test_runtime_launcher`](tests/test_runtime_launcher.rs) executa consumidores reais `.deb`, `.rpm` e ALPM com o mesmo provedor ELF, prova que dois comandos podem manter provedores diferentes com o mesmo SONAME e verifica que a remoção do provedor não quebra os consumidores.
- [`docs/support-matrix.md`](docs/support-matrix.md) lista somente combinações com fixture reproduzível.

O plano completo, incluindo aquisição, publicação por geração, recuperação, retenção e os gates ainda pendentes, está em [`docs/install-implementation-plan.md`](docs/install-implementation-plan.md).

## Superfície implementada nesta revisão

Além do caminho de instalação, o checkout agora contém os contratos operacionais de upgrade,
rollback, coleta conservadora, diagnóstico, perfis e execução por comando. A CLI expõe
`pkg upgrade`, `pkg doctor`, `pkg gc`, `pkg profile`, `pkg query-command`, `pkg run`,
`pkg rollback`, `pkg migrate` e `pkg mcp`; os caminhos de saída estruturada usam `--json`
e os erros principais preservam códigos de processo estáveis.

O diagnóstico possui também o modo opt-in `pkg doctor --repair`, que reconcilia
transações incompletas conhecidas e repete a inspeção sem tocar em conteúdo de
propriedade incerta.

Os testes também cobrem a recuperação após failpoints de promoção, ativação e commit,
remoção segura de perfil, limpeza de manifests de runtime órfãos, handshake MCP e upgrade
aplicado a partir de cache verificado. Isso fecha a fatia executável do plano; os gates de
matriz de hosts, extensões nativas, configuração mutável e aprovação formal G1--G10 ainda
precisam de evidência específica antes de uma declaração de suporte mais ampla.
