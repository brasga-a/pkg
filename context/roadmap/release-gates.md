# Release Gates

Release gates define the evidence required to declare a milestone complete. A feature being implemented is not enough: its milestone remains incomplete until every applicable gate below passes.

## Gate M0 — Foundation

Required evidence:

- the root Rust 2024 workspace manifest is valid;
- `crates/pkg-cli` and `crates/pkg-core` compile from a clean checkout;
- the full future crate split is not pre-created as empty speculative architecture;
- baseline workspace dependencies are centralized and limited to M0/M1 needs;
- later-milestone dependencies such as `tokio`, `reqwest`, `rpm`, and solver-specific crates are not introduced without current use;
- `Cargo.lock` is committed;
- `rust-toolchain.toml` includes the required formatter/linter components;
- `pkg --help` and `pkg --version` execute successfully;
- `cargo check --workspace` passes;
- `cargo test --workspace` passes;
- `cargo fmt --check` passes;
- `cargo clippy --workspace --all-targets -- -D warnings` passes;
- CI executes the same baseline checks on Linux;
- no M1 package-management behavior, host mutation, package script execution, or native package-manager integration is introduced merely as scaffolding.

Primary authority: **ADR-001; DEC-001; project/architecture.md.**

## Gate M1-A — Parser

Required evidence:

- malformed/truncated `.deb` corpus is rejected safely;
- extraction rejects absolute paths, `..` traversal, unsafe symlink/hardlink escapes, and configured expansion limits;
- maintainer scripts are inventoried but never executed;
- architecture mismatch is detected before promotion;
- normalized metadata is produced without invoking `dpkg`.

Primary authority: **ADR-005, ADR-011, ADR-017; INV-003, INV-004.**

## Gate M1-B — Store

Required evidence:

- install/remove is idempotent for supported fixtures;
- side-by-side store objects do not overwrite each other;
- profile activation is separate from payload storage;
- command-name conflicts fail explicitly;
- removal deletes only pkg-owned content;
- package payload never lands directly in `/`.

Primary authority: **ADR-003, ADR-010, ADR-014; INV-002, INV-006, INV-014, INV-015.**

## Gate M1-C — Transaction

Required evidence:

- forced interruption at transaction phases is detectable/recoverable;
- one-writer locking prevents concurrent state mutation races;
- staging/promotion/state reconciliation produces a known state after restart;
- install planning is side-effect free;
- native package-manager databases remain untouched.

Primary authority: **ADR-004, ADR-006, ADR-012; INV-001, INV-010, INV-011, INV-012, INV-013.**

## Gate M2 — Repository

Required evidence:

- repository metadata trust is verified according to the source ecosystem;
- artifact digest/signature failures fail closed;
- metadata freshness is reported separately from authenticity;
- failed refresh leaves the previous valid snapshot active;
- artifact cache identity is digest/validated-identity based rather than URL-only;
- offline/stale metadata behavior is explicit;
- remote installation reuses the M1 transaction path rather than introducing a second mutation path.

Primary authority: **ADR-002, ADR-007, ADR-008, ADR-013, ADR-016, ADR-018; INV-005, INV-016, INV-017, INV-018.**

## Gate M3 — Resolver

Required evidence:

- Debian, RPM, and ALPM source version semantics are preserved;
- normalized constraint IR represents required dependency/capability/conflict fixtures;
- solver/provider failures produce a human-readable explanation chain;
- compatibility tests reject package-name-only false equivalence;
- ELF/SONAME/ABI evidence can invalidate a nominal package-name match;
- RPM and ALPM adapters enter the same normalized planning model without leaking source-specific types into the core solver contract.

Primary authority: **ADR-005, ADR-009, ADR-016, ADR-017; INV-007, INV-008, INV-009, INV-020.**

## Gate M4 — Desktop integration

Required evidence:

- desktop entries, icons, and any accepted MIME actions are represented as typed integration operations;
- every host-visible action has pkg ownership/evidence sufficient for deterministic reversal;
- uninstall reverses only pkg-owned integrations;
- invalid or conflicting desktop integration fails closed;
- no arbitrary maintainer script is enabled to make desktop integration work;
- no system service, system user/group, native package DB, or system-critical path is modified;
- interrupted integration transactions recover to a known state.

Primary authority: **ADR-010, ADR-011, ADR-014, ADR-018; INV-003, INV-006, INV-014, INV-015, INV-018, INV-019.**

## Gate M5 / v1 — Hardening

Required evidence:

- profile generation switching is atomic and rollback restores a previously valid generation;
- upgrade uses the same staged transaction model and survives forced interruption;
- GC uses reachability/ownership evidence and never deletes uncertain or foreign content;
- `pkg doctor` detects incomplete transactions and DB/store/profile divergence;
- archive/parser/metadata fuzz targets run against a maintained hostile corpus; the
  reproducible smoke result is recorded in [docs/fuzzing-baseline-2026-09-17.md](../../docs/fuzzing-baseline-2026-09-17.md);
- threat model/security review finds no unresolved blocker against current invariants;
- native package databases remain untouched implicitly across install/update/upgrade/remove/GC flows;
- DEC-021 and DEC-022 are explicitly excluded from the v1 support claim by ADR-020;
- exact supported package/host tuples are documented;
- benchmark thresholds are recorded from reproducible CI hardware for repository parsing, solving, extraction, activation, and recovery.

Primary authority: **ADR-003, ADR-004, ADR-008, ADR-010, ADR-012, ADR-013, ADR-015; INV-001, INV-005, INV-006, INV-011, INV-012, INV-013, INV-014, INV-016, INV-017, INV-020.**
