# Milestones

This roadmap turns the accepted architecture into incremental implementation slices. A milestone is complete only when its listed tasks are implemented **and** every referenced release gate passes.

The governing hierarchy remains: accepted ADRs → invariants → requirements → canonical design → roadmap. If a milestone conflicts with an accepted ADR or invariant, the milestone must be corrected rather than worked around in code.

---

## M1 — Local artifact kernel

### Objective

Prove the smallest safe end-to-end installation path for a supported local `.deb`: parse it without `dpkg`, plan the install, extract into a pkg-owned user-space store, activate its executable, persist state, recover interrupted transactions, and remove it deterministically.

### Governing decisions

- **DEC-001** — Rust 2024 implementation.
- **DEC-003** — pkg-owned isolated store.
- **DEC-004** — never mutate native package databases implicitly.
- **DEC-005** — format adapters normalize artifacts.
- **DEC-006** — SQLite state database.
- **DEC-010** — profile-based binary activation.
- **DEC-011** — maintainer scripts default-deny.
- **DEC-012** — durable staged transactions and recovery.
- **DEC-014** — rootless user installation by default.
- **DEC-015** — stable CLI nouns/verbs; internal API remains unstable.
- **DEC-017** — format implementation starts with `.deb`.

### Required invariants

- **INV-001** — do not silently mutate `dpkg`, RPM, or libalpm databases.
- **INV-002** — never extract foreign payloads directly into `/`.
- **INV-003** — lifecycle scripts are default-deny.
- **INV-004** — extraction rejects absolute paths, traversal, and escaping links.
- **INV-005** — promoted store objects have verified identity and normalized metadata.
- **INV-006** — store content and activation state remain separate.
- **INV-010** — install planning is side-effect free.
- **INV-011** — only the transaction executor mutates package state.
- **INV-012** — one writer lock protects mutable pkg state.
- **INV-013** — interrupted transactions are detectable on startup.
- **INV-014** — removal is ownership-based.
- **INV-015** — activation conflicts are explicit.
- **INV-019** — system-critical packages are rejected by default policy.

### Governing ADRs

- [ADR-001 — Rust 2024 implementation](../adr/ADR-001-rust-2024-implementation.md)
- [ADR-003 — pkg-owned isolated store](../adr/ADR-003-pkg-owned-isolated-store.md)
- [ADR-004 — no implicit native package database mutation](../adr/ADR-004-no-implicit-native-package-database-mutation.md)
- [ADR-005 — artifact adapter boundary](../adr/ADR-005-artifact-adapter-boundary.md)
- [ADR-006 — SQLite local state](../adr/ADR-006-sqlite-local-state.md)
- [ADR-010 — profile-based binary activation](../adr/ADR-010-profile-based-binary-activation.md)
- [ADR-011 — maintainer scripts default-deny](../adr/ADR-011-maintainer-scripts-default-deny.md)
- [ADR-012 — durable staged transactions](../adr/ADR-012-durable-staged-transactions.md)
- [ADR-014 — rootless default](../adr/ADR-014-rootless-default.md)
- [ADR-015 — CLI contract](../adr/ADR-015-cli-contract.md)
- [ADR-017 — format support order](../adr/ADR-017-format-support-order.md)

### Small tasks

- [ ] Create the initial Rust 2024 crate/module layout and CLI entry point.
- [ ] Define package/domain IDs, versions, architectures, artifact identity, install plan, transaction ID, and installed-package records.
- [ ] Implement `.deb` probing and metadata parsing without invoking `dpkg`.
- [ ] Parse `control.tar.*` and inventory maintainer scripts without executing them.
- [ ] Implement hardened `data.tar.*` extraction into a staging directory.
- [ ] Reject path traversal, absolute paths, unsafe symlinks/hardlinks, malformed archives, and extraction-limit violations.
- [ ] Implement basic host facts for Linux x86_64 and static ELF dependency inspection.
- [ ] Create the rootless store/profile/state layout.
- [ ] Create SQLite schema and migrations for artifacts, packages, store objects, activations, and transactions.
- [ ] Add the single-writer process/file lock.
- [ ] Implement `InstallPlan` and `--dry-run` with zero host mutation.
- [ ] Implement staged install → verification → atomic promotion → profile activation → state commit.
- [ ] Implement startup transaction recovery and DB/store reconciliation.
- [ ] Implement profile binary symlink/wrapper activation with explicit command-conflict errors.
- [ ] Implement `pkg install <local.deb>`, `pkg list`, `pkg info`, and `pkg remove` for the supported fixture class.
- [ ] Add deterministic uninstall and repeated install/remove idempotence tests.
- [ ] Add fixtures for malformed archives, scripts, architecture mismatch, interrupted transactions, and activation conflicts.

### Release gates

- [Gate M1-A — Parser](release-gates.md#gate-m1-a--parser)
- [Gate M1-B — Store](release-gates.md#gate-m1-b--store)
- [Gate M1-C — Transaction](release-gates.md#gate-m1-c--transaction)

### Exit criteria

M1 is complete when a supported local `.deb` can be installed, inspected, executed through the selected pkg profile, removed, and recovered after forced interruption without writing package payloads into `/`, executing maintainer scripts, or altering native package-manager state.

---

## M2 — Remote catalog and trust

### Objective

Extend the local kernel into a remote package workflow: synchronize Debian repository metadata, preserve source-specific trust evidence, search normalized snapshots, download artifacts on demand, verify them, and feed the same M1 transaction path.

### Governing decisions

- **DEC-002** — Tokio for asynchronous network/process orchestration.
- **DEC-007** — repository metadata normalized into immutable snapshots.
- **DEC-008** — trust evidence remains source-specific.
- **DEC-013** — artifact cache is digest-addressed.
- **DEC-016** — cross-distro compatibility uses capabilities, not package names alone.
- **DEC-018** — host integration is explicit and reversible.
- **DEC-017** — Debian remains the first repository ecosystem.

### Required invariants

- **INV-005** — promoted content has verified identity.
- **INV-007** — package-name equality does not imply capability equality.
- **INV-009** — ABI compatibility uses evidence, not name mapping alone.
- **INV-010** — planning remains side-effect free.
- **INV-016** — metadata freshness and artifact authenticity are separate trust dimensions.
- **INV-017** — cache identity is validated/digest-based, not URL-only.
- **INV-018** — unsupported host integration fails closed.
- **INV-020** — resolution failures retain an explanation chain.

### Governing ADRs

- [ADR-002 — Tokio runtime](../adr/ADR-002-tokio-runtime.md)
- [ADR-007 — repository snapshots](../adr/ADR-007-repository-snapshots.md)
- [ADR-008 — source-specific trust evidence](../adr/ADR-008-source-specific-trust-evidence.md)
- [ADR-013 — digest-addressed artifact cache](../adr/ADR-013-digest-addressed-artifact-cache.md)
- [ADR-016 — capability-based cross-distro compatibility](../adr/ADR-016-capability-based-cross-distro-compatibility.md)
- [ADR-017 — format support order](../adr/ADR-017-format-support-order.md)
- [ADR-018 — explicit host integration](../adr/ADR-018-explicit-host-integration.md)

### Small tasks

- [ ] Add repository configuration loading and stable repository IDs.
- [ ] Implement bounded HTTP downloads with Tokio/reqwest.
- [ ] Separate metadata cache, artifact cache, and temporary downloads.
- [ ] Implement the first Debian repository metadata adapter.
- [ ] Verify repository metadata using Debian-compatible trust evidence.
- [ ] Normalize repository packages into immutable local snapshots.
- [ ] Atomically switch the active snapshot only after successful parse/verification.
- [ ] Keep the previous valid snapshot active when refresh fails.
- [ ] Implement digest-addressed artifact caching and mismatch rejection.
- [ ] Add `pkg repo list`, `pkg repo add`, and `pkg update` behavior.
- [ ] Implement repository-backed `pkg search` and `pkg info`.
- [ ] Implement remote `pkg install <name>` using the M1 transaction executor.
- [ ] Add explicit offline/stale-metadata behavior.
- [ ] Add tests for bad signatures, bad digests, interrupted downloads, stale snapshots, and failed refresh rollback.

### Release gate

- [Gate M2 — Repository](release-gates.md#gate-m2--repository)

### Exit criteria

M2 is complete when pkg can refresh a configured Debian repository, search its last valid normalized snapshot, download a selected artifact, reject invalid trust/integrity evidence, and install a supported package through the existing isolated transaction model.

---

## M3 — Cross-distro expansion

### Objective

Generalize the artifact/repository boundary to RPM and ALPM, introduce a normalized dependency/capability constraint IR, and prove that resolution can explain compatibility across ecosystems without treating distro package names as equivalent.

### Governing decisions

- **DEC-009** — dependency solver sits behind a normalized constraint IR.
- **DEC-016** — compatibility is capability/evidence based.
- **DEC-017** — support order is `.deb` → RPM → ALPM.
- **DEC-020** — native host dependency provider remains deferred; do not make it an implicit requirement.
- **DEC-021** — pkg-native repository signing is still open and is not required for RPM/ALPM interoperability.

### Required invariants

- **INV-007** — package-name equality is insufficient.
- **INV-008** — preserve source-distro version syntax.
- **INV-009** — libc/ELF ABI compatibility requires evidence.
- **INV-016** — freshness and authenticity remain separate.
- **INV-018** — unsupported integration fails closed.
- **INV-020** — unsatisfied dependency results include an explanation chain.

### Governing ADRs

- [ADR-005 — artifact adapter boundary](../adr/ADR-005-artifact-adapter-boundary.md)
- [ADR-008 — source-specific trust evidence](../adr/ADR-008-source-specific-trust-evidence.md)
- [ADR-009 — normalized solver IR](../adr/ADR-009-normalized-solver-ir.md)
- [ADR-016 — capability-based cross-distro compatibility](../adr/ADR-016-capability-based-cross-distro-compatibility.md)
- [ADR-017 — format support order](../adr/ADR-017-format-support-order.md)

### Small tasks

- [ ] Define the normalized constraint IR for all-of, any-of, capability, version, architecture, and conflicts.
- [ ] Preserve original Debian/RPM/ALPM expressions alongside normalized constraints for diagnostics.
- [ ] Implement RPM artifact probing, metadata normalization, payload extraction, and scriptlet inventory.
- [ ] Implement the first RPM repository metadata adapter.
- [ ] Implement ALPM `.pkg.tar.zst` parsing and `.PKGINFO` normalization.
- [ ] Implement the first ALPM repository database adapter.
- [ ] Normalize package `Provides`, executable capabilities, and relevant ELF/SONAME capabilities.
- [ ] Build provider-selection logic over package closure plus observed host capabilities.
- [ ] Evaluate a PubGrub-class solver behind the IR instead of coupling source adapters to solver types.
- [ ] Preserve ecosystem-specific version ordering rather than coercing versions to SemVer.
- [ ] Produce human-readable unsatisfied/conflict explanation chains.
- [ ] Build a compatibility corpus spanning representative Debian, RPM, and Arch packages.
- [ ] Add regression tests for false package-name equivalence and ABI mismatch.

### Release gate

- [Gate M3 — Resolver](release-gates.md#gate-m3--resolver)

### Exit criteria

M3 is complete when supported package classes from Debian, RPM, and ALPM can enter the same normalized planning model, resolver failures are explainable, source version semantics remain intact, and the compatibility corpus demonstrates that pkg does not claim support from name matching alone.

---

## M4 — Desktop application integration

### Objective

Add a narrow, reversible user-space host-integration layer for desktop applications without reopening arbitrary maintainer-script execution or system-wide ownership.

### Governing decisions

- **DEC-010** — store content and profile activation remain separate.
- **DEC-011** — lifecycle scripts remain default-deny.
- **DEC-014** — user/rootless operation remains the default.
- **DEC-015** — integrations surface through explicit CLI behavior rather than unstable hidden side effects.
- **DEC-018** — host integration must be typed, explicit, and reversible.

### Required invariants

- **INV-003** — arbitrary lifecycle scripts remain disabled.
- **INV-006** — store payload and activation/integration state are separate.
- **INV-014** — removal is ownership-based.
- **INV-015** — activation conflicts are explicit.
- **INV-018** — unsupported host integration fails closed.
- **INV-019** — system-critical packages remain rejected by default.

### Governing ADRs

- [ADR-010 — profile-based binary activation](../adr/ADR-010-profile-based-binary-activation.md)
- [ADR-011 — maintainer scripts default-deny](../adr/ADR-011-maintainer-scripts-default-deny.md)
- [ADR-014 — rootless default](../adr/ADR-014-rootless-default.md)
- [ADR-015 — CLI contract](../adr/ADR-015-cli-contract.md)
- [ADR-018 — explicit host integration](../adr/ADR-018-explicit-host-integration.md)

### Small tasks

- [ ] Define typed integration actions for user desktop entries, icons, and safe MIME registration.
- [ ] Persist ownership/evidence for every host-visible integration action.
- [ ] Implement desktop-entry validation and deterministic rewriting where required by store relocation.
- [ ] Implement user icon activation without system-wide writes.
- [ ] Implement safe MIME integration only for the accepted user-space subset.
- [ ] Extend transaction planning so integrations appear before mutation and participate in recovery.
- [ ] Reverse integrations during uninstall using ownership records rather than path heuristics.
- [ ] Expand relocation inspection for desktop application paths and data directories.
- [ ] Add fixtures for missing/invalid desktop files, conflicting desktop IDs, icons, and MIME declarations.
- [ ] Add a representative GUI application compatibility corpus.
- [ ] Verify that no system service, system user/group, package DB, or system-critical path is mutated.

### Release gate

- [Gate M4 — Desktop integration](release-gates.md#gate-m4--desktop-integration)

### Exit criteria

M4 is complete when supported desktop applications can be activated and removed entirely through typed user-space integration actions, with every host-visible mutation recorded, reversible, and covered by transaction recovery.

---

## M5 — v1 hardening

### Objective

Turn the proven package/store/repository model into a defensible v1 by adding lifecycle hardening: generations, upgrades, rollback, garbage collection, recovery tooling, fuzzing, security review, benchmarks, and a documented support matrix.

### Governing decisions

Accepted foundations:

- **DEC-003** — pkg-owned isolated store.
- **DEC-004** — no implicit native package DB mutation.
- **DEC-008** — source-specific trust evidence.
- **DEC-010** — profile-based activation.
- **DEC-012** — durable staged transactions.
- **DEC-013** — digest-addressed artifact cache.
- **DEC-015** — stable CLI contract, unstable internal API.

Open decisions that must be resolved before their dependent v1 work is considered final:

- **DEC-021** — pkg-native repository signing format.
- **DEC-022** — content-addressed store vs artifact-derived store IDs.

### Required invariants

- **INV-001** — native package databases remain untouched implicitly.
- **INV-005** — store identity remains verified.
- **INV-006** — store and activation remain separate.
- **INV-011** — transaction executor is the state mutation authority.
- **INV-012** — one writer protects mutable state.
- **INV-013** — interrupted operations remain detectable/recoverable.
- **INV-014** — removal/GC is ownership based.
- **INV-016** — freshness and authenticity remain distinct.
- **INV-017** — cache remains identity/digest based.
- **INV-020** — resolution failures stay explainable.

### Governing ADRs

- [ADR-003 — pkg-owned isolated store](../adr/ADR-003-pkg-owned-isolated-store.md)
- [ADR-004 — no implicit native package database mutation](../adr/ADR-004-no-implicit-native-package-database-mutation.md)
- [ADR-008 — source-specific trust evidence](../adr/ADR-008-source-specific-trust-evidence.md)
- [ADR-010 — profile-based binary activation](../adr/ADR-010-profile-based-binary-activation.md)
- [ADR-012 — durable staged transactions](../adr/ADR-012-durable-staged-transactions.md)
- [ADR-013 — digest-addressed artifact cache](../adr/ADR-013-digest-addressed-artifact-cache.md)
- [ADR-015 — CLI contract](../adr/ADR-015-cli-contract.md)

New ADRs are required before closing any work that resolves **DEC-021** or **DEC-022**.

### Small tasks

- [ ] Model profile generations as immutable activation snapshots.
- [ ] Implement atomic generation switching.
- [ ] Implement `pkg upgrade --dry-run` and transactional upgrade execution.
- [ ] Implement rollback to a retained valid generation.
- [ ] Build reachability analysis for store objects, retained generations, pins, and incomplete transactions.
- [ ] Implement `pkg gc --dry-run` before destructive GC.
- [ ] Garbage-collect only provably unreachable pkg-owned content.
- [ ] Implement `pkg doctor` checks for DB/store/profile/transaction consistency.
- [ ] Add conservative repair paths for known pkg-owned inconsistent state.
- [ ] Resolve DEC-021 with an ADR before declaring a pkg-native signing format stable.
- [ ] Resolve DEC-022 with an ADR before declaring final store identity semantics stable.
- [ ] Add archive/parser/metadata fuzz targets and a hostile corpus.
- [ ] Run transaction fault injection across install, upgrade, activation, rollback, and GC.
- [ ] Produce benchmark baselines for repository parsing, solving, extraction, activation, and recovery.
- [ ] Document exact support tuples/package classes instead of broad “all Linux packages” claims.
- [ ] Perform a security/threat-model review against current invariants and release gates.

### Release gate

- [Gate M5 / v1 — Hardening](release-gates.md#gate-m5--v1--hardening)

### Exit criteria

M5 is complete only when the v1 release gate passes, open v1-blocking decisions are resolved by ADRs, upgrade/rollback/GC/recovery are proven under fault injection, and the public support matrix accurately reflects tested package classes and host tuples.
