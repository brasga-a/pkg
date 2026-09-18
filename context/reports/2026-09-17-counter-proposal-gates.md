# Contra proposta rootless: relatório dos gates de execução

Este relatório liga cada gate da [contra proposta](../design/rootless-execution-counter-proposal.md)
a uma evidência reproduzível no checkout. A matriz é deliberadamente limitada ao
host observado no CI local: Linux x86_64 com glibc e loader do host disponível.
Uma linha fora dessa matriz continua sendo uma combinação não verificada.

## Resultado

| Gate | Estado no escopo declarado | Evidência principal | Limite explícito |
|---|---|---|---|
| G1 — planejamento puro | Validado | `tests/gate_m1_c_transaction.rs::test_dry_run_zero_host_mutation`; `tests/test_cli_json.rs::uncached_remote_dry_run_reports_missing_input_without_mutation`; `tests/test_cli_json.rs::local_install_json_contains_the_realized_plan` | Dry-run remoto não adquire artefatos; mudança de fonte exige novo plano. |
| G2 — evidência | Validado | `tests/gate_m1_a_parser.rs`; `tests/gate_m3_resolver.rs::test_gate_m3_elf_abi_evidence_invalidates_nominal_match`; `tests/review_regressions.rs::invalid_elf_and_missing_libraries_fail_before_promotion`; `tests/test_runtime_launcher.rs::unsupported_interpreter_family_is_rejected_before_activation`; corpus GUI em `crates/pkg-core/fixtures/gui` | Scripts de ciclo de vida são apenas inventariados; loaders estrangeiros são rejeitados. |
| G3 — execução real | Validado para x86_64/glibc | `tests/test_runtime_launcher.rs::real_elf_provider_is_reused_across_deb_rpm_and_alpm_consumers` | A fixture prova `.deb` como provedor e consumidores RPM/ALPM; não certifica distribuições arbitrárias. |
| G4 — fidelidade do loader | Validado para a estratégia de runner nativo | `tests/test_runtime_launcher.rs::test_native_runner_uses_promoted_package_library_view`; `::native_runner_resolves_a_real_transitive_elf_closure`; `::package_rpath_cannot_escape_staging_root` | Loader estrangeiro, musl e arquiteturas não presentes na matriz ficam fora do suporte. |
| G5 — comandos independentes | Validado para comandos ELF estáticos | `tests/test_runtime_launcher.rs::command_runtimes_keep_independent_same_soname_providers`; `::installing_an_unrelated_package_preserves_existing_runtime_commands` | Plugins dinâmicos e helpers que não aparecem no contrato ELF não são admitidos silenciosamente. |
| G6 — semântica de runtime | Validado para adapters declarados | `tests/test_runtime_launcher.rs::generic_interpreter_adapters_preserve_arguments`; `::test_python_runtime_launcher_and_scoped_pythonpath`; `::test_native_runner_uses_promoted_package_library_view`; `::invalid_native_extension_is_rejected_before_activation`; `::unsupported_interpreter_family_is_rejected_before_activation` | Python nativo é inspecionado; famílias sem adapter e extensões inválidas falham antes da ativação. |
| G7 — integridade de adaptações | Validado | `crates/pkg-core/src/domain/relocation.rs` (testes de `.ucf` e FHS); `crates/pkg-core/src/store/layout.rs::derivation_id_changes_when_recipe_or_prefix_changes`; `tests/review_regressions.rs::unmanaged_and_modified_activation_paths_are_preserved` | Receitas novas exigem versão e entradas/saídas explícitas no manifesto. |
| G8 — recuperação transacional | Validado | `tests/test_fault_injection.rs`; `tests/review_regressions.rs` (upgrade, remove, rollback e primeira publicação); recuperação de integração em `transaction::recovery` | A publicação é por geração; leitores nunca usam a staging diretamente. |
| G9 — retenção e migração | Validado para referências conhecidas | `tests/test_runtime_launcher.rs::real_elf_provider_is_reused_across_deb_rpm_and_alpm_consumers`; `tests/test_profiles.rs`; `tests/review_regressions.rs::legacy_profile_migration_is_marked_unverified` | Evidência legada incompleta permanece `unverified`; não há coleta automática de objetos em uso por processos externos. |
| G10 — suporte reportado | Validado | `docs/support-matrix.md`; JSON de instalação em `tests/test_cli_json.rs`; explicações do resolver em `tests/gate_m3_resolver.rs`; baseline em `docs/benchmark-baseline-2026-09-17.md` | Combinações ausentes da matriz não são prometidas. |

## Como reproduzir

```text
cargo test --workspace --all-features --no-fail-fast
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
benchmarks/run-gates.sh --runs 10 --warmup 2 --output-dir /tmp/pkg-bench-gates
```

O último ciclo foi executado em 17/09/2026. A suíte completa passou, o linter
passou e os cinco gates de benchmark terminaram com código zero. Os números
brutos estão em [`benchmarks/results/gates/results.json`](../../benchmarks/results/gates/results.json).

## Decisão de escopo

Os gates são considerados fechados para a matriz declarada porque cada caminho
de publicação possui evidência persistida, revalidação no lançamento e teste de
falha correspondente. Isso não transforma um resultado x86_64/glibc em suporte
para musl, outra arquitetura, loader estrangeiro ou plugin dinâmico. Para
admitir qualquer uma dessas classes é necessário adicionar um adapter, fixtures
reais e uma nova linha em `docs/support-matrix.md` antes de alterar o estado do
gate.
