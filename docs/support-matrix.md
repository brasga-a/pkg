# Matriz de suporte executável

Esta matriz registra apenas combinações que têm evidência no checkout atual.
Uma combinação ausente continua sem suporte verificado; o nome do formato não
é uma prova de compatibilidade.

| Host observado | libc/loader | Consumidor → provedor | Estratégia | Evidência |
|---|---|---|---|---|
| Ubuntu 26.04 x86_64 | glibc, `/lib64/ld-linux-x86-64.so.2` | `.deb` → `.deb` | runner nativo e view por comando | `tests/test_runtime_launcher.rs::test_native_runner_uses_promoted_package_library_view` |
| Ubuntu 26.04 x86_64 | glibc, `/lib64/ld-linux-x86-64.so.2` | provedor `.deb`; consumidores `.rpm` e `.alpm` (mesmo SONAME) | runner nativo, três consumidores com o mesmo provedor | `tests/test_runtime_launcher.rs::real_elf_provider_is_reused_across_deb_rpm_and_alpm_consumers` |
| Ubuntu 26.04 x86_64 | glibc, `/lib64/ld-linux-x86-64.so.2` | dois consumidores `.rpm`/ALPM com provedores `.deb` diferentes e o mesmo SONAME | views independentes por comando | `tests/test_runtime_launcher.rs::command_runtimes_keep_independent_same_soname_providers` |
| Ubuntu 26.04 x86_64 | `/usr/bin/python3` disponível | ALPM Python puro, com `site-packages` | adaptador de interpretador e `PYTHONPATH` por comando | `tests/test_runtime_launcher.rs::test_python_runtime_launcher_and_scoped_pythonpath` |
| Ubuntu 26.04 x86_64 | `/usr/bin/perl` e `/usr/bin/node` disponíveis | scripts puros Debian com shebang Perl e `/usr/bin/env node` | adaptador de interpretador com argumentos preservados | `tests/test_runtime_launcher.rs::generic_interpreter_adapters_preserve_arguments` |
| Qualquer host sem o bootstrap estático compilado | — | ELF dinâmico | rejeitado antes da publicação | contrato de `native_runner_bytes()` |

O teste cross-format prova a reutilização de uma biblioteca ELF real entre
artefatos Debian, RPM e ALPM. Ele não certifica que bibliotecas arbitrárias de
distribuições diferentes sejam compatíveis: SONAME, máquina, classe, símbolos,
loader e dependências transitivas ainda precisam coincidir.

## Fora da matriz

- aarch64, riscv64, 32-bit e combinações musl não têm fixture executada neste
  checkout;
- loader estrangeiro presente apenas dentro do payload é rejeitado; a chamada
  por loader explícito permanece desabilitada até possuir fixtures de argumento,
  `RUNPATH`, sinais e reexecução;
- R e Ruby não têm intérprete disponível na matriz atual. Perl e Node têm
  adaptador para scripts puros; extensões nativas desses interpretadores ainda
  exigem fixtures e validação próprias;
- integrações desktop, serviços privilegiados, bancos dos gerenciadores nativos
  e scripts de mantenedor continuam fora da unidade de publicação.

Novas linhas entram aqui somente com um teste reproduzível e com a versão do
host/libc/loader registrada no resultado. A matriz não transforma uma execução
bem-sucedida em outra arquitetura ou distribuição em uma promessa geral.
