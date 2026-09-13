# Implementation Handoff — Milestone 1

## Authorized goal

Prove the smallest safe vertical slice:

```bash
pkg install ./fixture.deb
pkg list
pkg remove fixture
```

The package is installed into a pkg-owned user store and its executable is exposed through a pkg-owned `bin` directory.

## Required architecture slice

```text
CLI
 -> ingest local path
 -> DebAdapter
 -> NormalizedPackage
 -> CompatibilityCheck
 -> InstallPlan
 -> StagingStore
 -> Verify
 -> AtomicPromote
 -> SQLite state
 -> BinLink activation
```

## Suggested initial Rust modules

```text
src/
├── cli/
├── domain/
├── format/
│   └── deb/
├── host/
├── planner/
├── store/
├── state/
├── transaction/
├── verify/
└── error.rs
```

## First types

```rust
PackageId
PackageName
PackageVersion
Architecture
ArtifactDigest
NormalizedPackage
Dependency
Capability
InstallPlan
StorePath
InstalledPackage
TransactionId
```

## Acceptance criteria

Milestone 1 is complete only if:

- fixture `.deb` metadata is parsed without invoking `dpkg`;
- payload extraction rejects traversal and unsafe links;
- installation never writes package payload into `/usr`;
- no maintainer script executes;
- a staged install is promoted atomically on the same filesystem;
- installed state includes artifact digest, version, architecture and file ownership;
- a binary can be launched through the pkg bin exposure;
- removal is deterministic and does not delete foreign paths;
- interruption tests leave either the old valid state or a recoverable transaction;
- concurrent state mutation is serialized by a process/file lock;
- tests prove native package-manager databases are untouched.

## Explicitly deferred

Remote repositories, `.rpm`, `.pkg.tar.zst`, generalized solving, signature trust roots, desktop integration and system service integration belong to later gates.
