# Architecture Critique

## Strong properties

The isolated-store model directly addresses the worst coexistence problem: file ownership conflicts with native package managers.

The explicit distinction between artifact parsing and package compatibility prevents a common conceptual failure where supporting `.deb` syntax is marketed as supporting Debian packages everywhere.

The transaction/profile split gives upgrades and uninstall a reversible structure.

## Weak points

### Relocation remains the central technical risk
Many packages embed absolute paths. The architecture can detect some cases but cannot generically repair them.

### Host capability detection can become a second package manager
If pkg tries to fully model every native package database and ABI, complexity explodes.

Recommendation: keep host evidence narrow and observable. Add native-query providers only when demanded by test packages.

### Solver sophistication can arrive too early
A universal SAT model is tempting. The first milestones should be driven by controlled fixtures and real repository corpora before committing to maximal expressiveness.

### Security must not be confused with isolation
Store separation protects ownership, not runtime authority. Applications are not sandboxed merely because they live under `~/.local/share/pkg/store`.

## Architectural stop conditions

Revisit the foundation if tests show:
- most desired vendor packages cannot run from an isolated root without invasive binary patching;
- capability mapping produces frequent false positives;
- host integration requires arbitrary scripts for ordinary applications;
- transaction recovery cannot be made deterministic without a daemon.
