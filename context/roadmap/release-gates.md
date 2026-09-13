# Release Gates

## Gate M1-A — Parser
- malformed package corpus passes;
- no archive escape;
- no scripts execute.

## Gate M1-B — Store
- install/remove idempotence;
- side-by-side versions;
- activation conflict tests.

## Gate M1-C — Transaction
- forced interruption recovery;
- writer lock tests;
- DB/store reconciliation.

## Gate M2 — Repository
- stale metadata fallback is explicit;
- signature/digest failures fail closed;
- previous valid snapshot survives failed sync.

## Gate M3 — Resolver
- source version semantics preserved;
- human-readable unsat explanation;
- no package-name-only false compatibility in test corpus.

## Gate v1
- threat model reviewed;
- fuzz corpus;
- no native package DB mutation;
- documented support matrix;
- benchmark thresholds established from CI hardware.
