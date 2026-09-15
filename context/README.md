# pkg Engineering Context

Status: **architecture proposal / pre-implementation**  
Canonical architecture date: **2026-09-12**

`pkg` is the temporary project name for a cross-distribution Linux package manager written in Rust.

The core architectural hypothesis is deliberately narrower than “make every distro package universally installable”:

> Treat `.deb`, `.rpm`, `.pkg.tar.zst`, tarballs and future formats as **input artifacts**, normalize their metadata, verify them, and materialize their payload into a **pkg-owned isolated store**. Do not pretend that Fedora or Arch are Debian, and do not mutate `dpkg`, RPM or libalpm databases behind the native package manager.

The first stable target is application/tool installation in user space. System foundations such as kernels, bootloaders, libc replacement, PAM, display managers, kernel modules and init-system ownership are explicitly outside the MVP.

## Source-of-truth hierarchy

When documents disagree, use this precedence:

1. accepted ADRs in `adr/`;
2. `project/invariants.md`;
3. `project/requirements.md`;
4. canonical `project/` and `design/` documents;
5. `roadmap/`;
6. `research/`, `*-analysis.md` and `*-proposal.md`;
7. raw/reference material in `sources/`.

`meta/decisions.md` is the decision ledger and must agree with all accepted ADRs.

## Start here

- Coding agents: `AGENTS.md`, then `implementation-handoff.md`.
- Product boundary: `project/vision.md`, `project/scope.md`, `project/requirements.md`.
- Architecture: `project/architecture.md`, `project/invariants.md`, `meta/decisions.md`.
- Package/install model: `design/package-model.md`, `design/store-layout.md`, `design/transaction-model.md`.
- Cross-distro behavior: `design/compatibility-model.md`, `design/cross-distro-installation.md`.
- Repository behavior: `design/repository-model.md`, `design/repository-sync.md`.
- CLI command contracts: `commands/README.md` and individual command documents in `commands/`.
- Security: `design/integrity-and-signatures.md`, `design/security.md`, `research/security-analysis.md`.
- Agent-first architecture: `design/agent-first-architecture.md`, `adr/ADR-019-agent-first-architecture-and-mcp-integration.md`.
- Delivery plan: `roadmap/mvp.md`, `roadmap/milestones.md`, `roadmap/release-gates.md`.

## Canonical versus analysis documents

Files named `*-analysis.md`, `*-proposal.md`, `architecture-options.md`, and critique/review files preserve reasoning. They are not implementation authority after an ADR and canonical design exist.

The initial implementation should optimize for **correctness, reversibility and coexistence with native package managers**, not broad compatibility claims.
