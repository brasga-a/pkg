# pkg

A universal, cross-distribution, rootless package manager for Linux — built for Humans and AI Agents.

> **Status:** beta / under active development.

`pkg` aims to provide one predictable installation interface across Linux distributions **without pretending Debian, Fedora and Arch are the same system**.

The core model treats `.deb`, RPM, `.pkg.tar.zst` and future package formats as input artifacts. Their metadata is normalized, compatibility is evaluated explicitly, and supported packages are materialized into a **pkg-owned isolated store** instead of being sprayed directly into `/usr` or silently registered in `dpkg`, RPM or libalpm.

## Why

Linux software distribution is fragmented across package formats, repository models, dependency vocabularies and distro policies. A vendor may ship only a `.deb`, only an RPM, an Arch package, or a tarball.

The project goal is to normalize **delivery and package management UX**, while preserving the technical boundaries that actually matter: ABI compatibility, lifecycle behavior, repository trust, file ownership and host integration.

## Quick Install

Install the latest release of `pkg` rootless into user-space:

```bash
curl -fsSL https://pkg.atlantic.sh/install | sh
```

Custom options:

```bash
curl -fsSL https://pkg.atlantic.sh/install | bash -s -- --dir ~/.local/bin --version 0.1.0-beta.1
```

```bash
pkg search ripgrep
pkg install ripgrep
pkg install ./vendor.deb
pkg info ripgrep
pkg remove ripgrep
```

## Architecture

```text
Package source / local artifact
            |
        Fetch / ingest
            |
       Format adapter
   (.deb / RPM / ALPM)
            |
     Normalized package
            |
   Resolver + compatibility
            |
        Install plan
            |
    Transaction executor
       /      |       \
    Store   State DB   Activation
```

The architecture is built around a few hard boundaries:

- package payloads are installed into a **pkg-owned isolated store**;
- native package-manager databases are not modified implicitly;
- foreign maintainer scripts are **default-deny**;
- dependency compatibility is based on capabilities/evidence, not package-name matching alone;
- planning is side-effect free;
- install/update/remove operations are transaction-oriented and recoverable;
- user-space/rootless installation is the default;
- dual interface contract: human-friendly terminal UX alongside machine-first `--json` and native Model Context Protocol (MCP) server for AI agents;
- host-visible integration must be explicit, typed and reversible.

See [`context/README.md`](context/README.md) for the full engineering context and source-of-truth hierarchy.

## Built for Humans and AI Agents

`pkg` is designed from the foundation for both human software engineers and autonomous AI agents (such as Google Antigravity, Claude Code, Cursor, Devin, and OpenHands):

- **Zero Root Privilege**: Runs 100% rootless in user-space. Agents cannot break system libraries, corrupt PAM/init configs, or brick the host operating system.
- **Fail-Safe Non-Interactive Execution**: Never hangs indefinitely waiting for input on `stdin` when running in headless/agent environments.
- **Machine-First Structured Contracts**: Full `--json` support across commands with stable schemas, exact binary path reporting, and clear exit codes.
- **Native Model Context Protocol (MCP)**: Directly integrates with agent tools via `pkg mcp` over standard I/O (see [`context/design/agent-first-architecture.md`](context/design/agent-first-architecture.md)).
- **Ephemeral Task Profiles**: Allows agents to spin up disposable workspaces (`pkg profile create <task>`) and drop them cleanly without leaving residual clutter on the user's system.

## Planned MVP

The first product claim is intentionally narrow:

> pkg can safely install a supported local application package into its own user-space store and expose its commands without handing file ownership to the host package manager.

Initial MVP scope:

- Linux x86_64;
- local `.deb` artifacts;
- package classes that do not require maintainer scripts;
- isolated store + activation profile + SQLite state;
- static ELF/host-library compatibility checks;
- deterministic install/remove/list/info;
- `--dry-run` install planning;
- transaction recovery.

The MVP does **not** claim universal `.deb` compatibility, RPM/Arch support, distro upgrades, system services, arbitrary maintainer scripts or universal dependency solving.

## Roadmap

| Milestone | Goal | Release gate |
|---|---|---|
| **M0 — Project foundation** | Rust workspace, initial `crates/`, root `Cargo.toml`, shared dependencies, CI, lint/test foundation | M0 Foundation |
| **M1 — Local artifact kernel** | Safe local `.deb` → normalized metadata → isolated store → activation → deterministic removal | M1-A Parser, M1-B Store, M1-C Transaction |
| **M2 — Remote catalog and trust** | Debian repository sync, immutable snapshots, trust verification, remote search/install | M2 Repository |
| **M3 — Cross-distro expansion** | RPM + ALPM adapters, normalized constraint IR, capability-based resolution | M3 Resolver |
| **M4 — Desktop integration** | Typed/reversible desktop entries, icons and safe user-space integration | M4 Desktop integration |
| **M5 — v1 hardening** | Generations, upgrade/rollback, GC, doctor, fuzzing, security review and support matrix | M5 / v1 Hardening |

Each milestone is expanded with its **governing decisions, invariants, ADRs, small implementation tasks, release gates and exit criteria** in [`context/roadmap/milestones.md`](context/roadmap/milestones.md).

Release evidence is defined separately in [`context/roadmap/release-gates.md`](context/roadmap/release-gates.md).

## CLI contract

The planned command surface currently includes:

```text
pkg install
pkg remove
pkg integrate <package>
pkg deintegrate <package>
pkg list
pkg info
pkg search
pkg repo sync
pkg update
pkg upgrade
pkg repo list
pkg repo add
pkg repo update # alias of pkg repo sync
pkg doctor
pkg gc
pkg profile create <task>
pkg profile list
pkg profile drop <task>
pkg query-command <command>
pkg mcp
```

Detailed behavior for each command lives in [`context/commands/`](context/commands/README.md).

## Engineering context

The repository carries an explicit context pack so implementation agents and contributors can distinguish architecture authority from research/proposals.

Start here:

- [`context/README.md`](context/README.md) — context navigation and authority hierarchy;
- [`context/AGENTS.md`](context/AGENTS.md) — implementation guidance;
- [`context/project/architecture.md`](context/project/architecture.md) — component architecture;
- [`context/project/invariants.md`](context/project/invariants.md) — rules implementation must not violate;
- [`context/meta/decisions.md`](context/meta/decisions.md) — decision ledger;
- [`context/adr/`](context/adr/README.md) — accepted Architecture Decision Records;
- [`context/roadmap/milestones.md`](context/roadmap/milestones.md) — implementation roadmap;
- [`context/roadmap/release-gates.md`](context/roadmap/release-gates.md) — evidence required to complete milestones.

### Source-of-truth order

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
Research / analysis / proposals
    ↓
Raw sources
```

If implementation conflicts with an invariant, the implementation is wrong unless the governing architecture is explicitly revised through the decision/ADR process.

## Non-goals

`pkg` is not initially trying to replace the operating system package manager for:

- kernel or bootloader management;
- libc replacement;
- PAM/NSS;
- init-system ownership;
- drivers/DKMS;
- distro upgrades;
- arbitrary system-wide service installation.

The first goal is narrower: make supported Linux applications easier to distribute and install across distributions **without corrupting the host package-management model**.
