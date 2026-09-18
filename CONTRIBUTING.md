# Contributing to pkg

Thanks for your interest in `pkg`.

`pkg` is an experimental cross-distribution package manager for Linux, written in Rust. Contributions are welcome, especially around package formats, repository handling, compatibility analysis, transactions, testing, benchmarking, documentation, and developer experience.

Because package management touches filesystem ownership, trust, and host compatibility, changes should favor correctness and explicit behavior over shortcuts.

## Before you start

Read the project context before making non-trivial changes:

- [`context/README.md`](context/README.md) — context navigation and authority hierarchy
- [`context/AGENTS.md`](context/AGENTS.md) — implementation guidance
- [`context/project/invariants.md`](context/project/invariants.md) — rules implementation must not violate
- [`context/project/architecture.md`](context/project/architecture.md) — architecture
- [`context/meta/decisions.md`](context/meta/decisions.md) — accepted and open decisions
- [`context/adr/`](context/adr/README.md) — Architecture Decision Records
- [`context/roadmap/milestones.md`](context/roadmap/milestones.md) — roadmap and milestone scope

The authority order is:

```text
Accepted ADRs
    ↓
Architectural invariants
    ↓
Requirements
    ↓
Canonical project/design docs
    ↓
Roadmap
    ↓
Research / proposals
```

If an implementation conflicts with an accepted invariant or ADR, do not silently work around it. Either change the implementation or propose an explicit architecture update.

## Current project boundaries

Keep these principles in mind:

- do not silently mutate `dpkg`, RPM, or libalpm package databases;
- do not extract foreign package payloads directly into `/`;
- maintainer scripts are default-deny unless explicitly supported by policy;
- rootless/user-space operation is the default;
- package names from different ecosystems are not assumed equivalent;
- install/remove/update operations should remain transaction-oriented;
- filesystem and network inputs must be treated as untrusted;
- host integration must be explicit and reversible.

Avoid expanding milestone scope just because an abstraction could support a future feature.

## Development setup

### Requirements

- Linux
- Rust `1.85+`
- Cargo
- Git

Clone the repository and build the workspace:

```bash
git clone https://github.com/brasga-a/pkg.git
cd pkg
cargo build --workspace
```

Run the CLI locally:

```bash
cargo run -p pkg-cli -- --help
```

For optimized builds:

```bash
cargo build --release
```

## Tests and quality checks

Before opening a pull request, run:

```bash
cargo fmt --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
```

Changes that touch archive parsing, filesystem mutation, repository metadata, transactions, package ownership, or compatibility logic should include regression tests for both the success path and relevant failure paths.

Important failure classes include:

- malformed or truncated archives;
- path traversal and link escapes;
- checksum or signature mismatch;
- activation conflicts;
- unsupported architecture or ELF requirements;
- interrupted transactions;
- concurrent state mutation;
- stale or hostile repository metadata;
- accidental native package-database mutation.

## Documentation changes

User-visible behavior and architecture documentation are part of the implementation.

When changing CLI semantics, update the corresponding document under [`context/commands/`](context/commands/README.md).

When changing architecture or a project invariant, update the relevant decision documentation and add or revise an ADR when required.

Do not describe roadmap features as already implemented.

## Pull requests

Keep pull requests focused. A good PR should:

1. explain the problem being solved;
2. describe the chosen approach;
3. identify affected invariants, decisions, or ADRs when relevant;
4. include tests for new behavior;
5. update documentation when behavior changes;
6. pass formatting, linting, build, and test checks.

For larger architectural changes, open an issue or discussion first so the design can be reviewed before implementation becomes expensive.

### Suggested PR description

```md
## Summary

What changed and why.

## Architecture / invariants

- Affected ADRs or invariants, if any

## Testing

- Tests added or updated
- Commands used to validate the change

## Notes

Known limitations, follow-up work, or compatibility concerns.
```

## Commit style

Prefer concise conventional-style commit messages:

```text
feat: add rpm metadata adapter
fix: reject symlink escape during extraction
perf: reduce repository snapshot lookup overhead
test: add interrupted install regression
docs: document repository trust model
refactor: isolate package activation logic
```

Keep commits understandable and avoid mixing unrelated changes when practical.

## Benchmarks

Performance changes should be measured with reproducible inputs and environment information.

Do not publish claims such as "X times faster" unless the compared operations perform sufficiently equivalent work and the methodology is documented.

Prefer reporting absolute latency, median/p95 where useful, peak memory, I/O, and the exact artifact/catalog used.

See [`benchmarks/`](benchmarks/) when working on performance-sensitive paths.

## Security-sensitive contributions

Package managers process untrusted artifacts and metadata. Be especially careful with:

- archive extraction;
- symlinks and hardlinks;
- filesystem path validation;
- signature and digest verification;
- subprocess execution;
- package-provided strings;
- repository responses;
- privilege boundaries.

Never construct shell commands directly from package metadata.

If you discover a security issue, avoid publishing exploit details in a public issue until a safe disclosure path has been agreed upon.

## Scope for contributors

Useful contribution areas include:

- `.deb`, RPM, and ALPM format adapters;
- repository parsing and trust verification;
- ELF and ABI compatibility analysis;
- dependency/capability modeling;
- transaction recovery;
- state/store correctness;
- CLI ergonomics;
- benchmarks and profiling;
- Linux distro compatibility testing;
- fixtures, fuzzing, and regression tests;
- documentation.

Small, well-tested contributions are welcome.

## License

By contributing to this repository, you agree that your contributions will be licensed under the repository's Apache-2.0 license.
