# Fuzzing

The standalone `fuzz/` crate contains libFuzzer targets for metadata parsing
and bounded payload extraction. It is intentionally outside the production
workspace so fuzz-only dependencies never enter the runtime dependency graph.

Install `cargo-fuzz` once, then run the targets from the repository root:

```bash
cargo install cargo-fuzz
cargo fuzz run metadata fuzz/corpus/metadata
cargo fuzz run extraction fuzz/corpus/extraction
```

Em ambientes que já têm o toolchain do projeto, o smoke gate reproduzível
também pode ser executado sem instalar `cargo-fuzz`:

```bash
fuzz/run-corpus.sh 100
```

Esse comando compila os dois alvos em release e executa 100 entradas por alvo
contra o corpus versionado. Em 17/09/2026 os dois alvos terminaram com código
zero e sem crash. A execução de release não habilita os hooks opcionais do
sanitizer; uma campanha com `cargo fuzz` continua sendo a validação ampliada
para o CI que disponibilizar esse toolchain.

The targets cap input and extraction sizes and treat parser errors as expected
outcomes. A panic, sanitizer finding or timeout is a release blocker and must
be added to the hostile corpus as a regression before the gate is reconsidered.
