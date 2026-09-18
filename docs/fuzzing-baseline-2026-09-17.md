# Baseline de fuzzing — 17/09/2026

Comando reproduzido:

```text
fuzz/run-corpus.sh 100
```

| Alvo | Corpus | Execuções | Resultado |
|---|---|---:|---|
| `metadata` | `fuzz/corpus/metadata` | 100 | código 0, sem crash |
| `extraction` | `fuzz/corpus/extraction` | 100 | código 0, sem crash |

Os alvos são compilados em release usando `libfuzzer-sys`. A imagem de execução
não forneceu os hooks opcionais do sanitizer (`__sanitizer_*`); isso foi
reportado pelo runtime como warning e não alterou o resultado do corpus. O CI
com `cargo fuzz` deve manter uma campanha de maior duração e anexar qualquer
novo input interessante ao corpus antes de promover a matriz de suporte.
