# AGENTS.md — pkg implementation guidance

## Project

`pkg` is a cross-distribution Linux package manager. It ingests package artifacts from multiple ecosystems and installs supported applications into a store it owns.

The project is **not** a universal replacement for `apt`, `dnf` or `pacman` in its first milestones. It must coexist with them.

## Core execution path

```text
CLI
 |
Command service
 |
Repository/catalog -------- Package source
 |                            |
Resolver                    Downloader
 |                            |
Compatibility planner ---- Verifier
             \              /
              Install Plan
                  |
             Transaction
            /      |       \
       Store    State DB   Host links
```

Format-specific knowledge ends at adapter boundaries:

```text
.deb --------\
.rpm ---------+--> ArtifactAdapter --> NormalizedPackage
.pkg.tar.zst -/
tarball ------/
```

## Architectural invariants

1. Native package-manager databases are never modified implicitly.
2. Store paths are pkg-owned and versioned; package payloads do not get sprayed directly into `/usr`.
3. Installation is plan-first and transaction-oriented.
4. Metadata from foreign package ecosystems is untrusted input.
5. Maintainer/install scripts are disabled by default in the MVP.
6. Dependency names from Debian, RPM and ALPM are not assumed equivalent.
7. Compatibility is expressed through normalized capabilities and verified host facts.
8. A successful extraction is not equal to a successful installation.
9. Every installed file in pkg-owned state has deterministic ownership.
10. Removal deletes only pkg-owned paths or explicit host links created by pkg.
11. Repository metadata and package payload integrity are verified independently.
12. User-space/rootless operation is the default.
13. Host integration is narrow, explicit and reversible.
14. No package may silently replace host libc, init, kernel, PAM, package database or boot components.
15. Unsupported lifecycle behavior yields `Unsupported`/`NeedsHostIntegration`, never guessed success.

## MVP scope

Implement:

- CLI skeleton;
- host detection;
- local `.deb` ingestion first;
- normalized package metadata;
- extraction into isolated store;
- ELF dependency inspection;
- state database;
- atomic exposure of executables through a pkg bin directory;
- deterministic uninstall;
- verification of checksums;
- dry-run/install plan;
- fixture packages and failure tests.

Do not initially implement:

- executing arbitrary Debian/RPM/ALPM maintainer scripts;
- replacing native system packages;
- kernel packages, drivers or DKMS;
- service installation;
- global `/usr` writes;
- full SAT compatibility across foreign repositories;
- automatic distro dependency-name translation;
- containers as the default execution model;
- root daemon;
- remote build farm;
- source builds;
- package publishing protocol.

## Coding rules

- Rust 2024.
- `unsafe` requires a platform-specific reason and a documented invariant.
- Network, parsing and filesystem layers must have bounded input sizes.
- Never build shell command strings from package metadata.
- Use argv arrays for subprocesses.
- Make install plans serializable internally for debugging, but do not declare a stable public wire format in the MVP.
- Keep adapters pure where possible: parse bytes -> normalized metadata/payload entries.
- Keep host mutation behind a small `HostIntegration` boundary.
- Lock mutable pkg state during transactions.
- Database commit must not claim filesystem success before filesystem promotion is complete.
- Prefer staging + atomic rename on the same filesystem.
- Record enough evidence to recover or clean a failed transaction.

## Required tests

- malformed/truncated archives;
- archive path traversal (`../`, absolute paths, symlink escapes);
- duplicate file paths;
- conflicting binary names;
- interrupted installs;
- checksum mismatch;
- unsupported architecture;
- unsatisfied ELF libraries;
- transaction recovery;
- uninstall idempotence;
- concurrent `pkg install`;
- stale repository metadata;
- package version side-by-side behavior;
- hostile package metadata sizes;
- scripts present but not executed;
- native package database remains unchanged.
## CLI command contracts

User-facing command behavior is documented under `commands/`. When implementation behavior changes a command's semantics, update the corresponding command document and any governing ADR/requirement in the same change.

