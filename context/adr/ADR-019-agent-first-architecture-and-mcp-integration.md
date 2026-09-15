# ADR-019: Agent-first architecture and native MCP integration

## Status

Accepted

## Context

Modern developer environments increasingly involve autonomous AI agents (such as Google Antigravity, Claude Code, Cursor, Devin, OpenHands, SWE-bench) operating on host systems and sandboxes. Traditional package managers (`apt`, `dnf`, `pacman`, `brew`, `nix`) present fundamental barriers to AI agent operation:
1. They require root (`sudo`) privileges, introducing severe security hazards for autonomous execution.
2. They rely on interactive TTY prompts (`[Y/n]`, debconf dialogs) that cause agent subprocesses to hang indefinitely on `stdin`.
3. They emit unstructured text, ANSI escape sequences, and terminal spinners that require fragile regex scraping by LLMs.
4. They are tightly coupled to specific host distributions, requiring agents to maintain distribution-specific logic.
5. They pollute host global state, complicating ephemeral task sandboxing.

`pkg` is uniquely positioned to solve these challenges through its rootless user-space store, cross-distro artifact normalization, and isolated profile model. To realize this, `pkg` must treat AI agents as first-class consumers alongside human operators.

## Decision

`pkg` adopts an **Agent-First Architecture** with five contractual pillars:

1. **Machine-First Structured Contracts (`--json` / `--jsonl`)**:
   Every CLI command (`install`, `remove`, `search`, `list`, `info`, `doctor`, `gc`, `profile`) must support a `--json` flag outputting deterministic, schema-stable JSON payloads containing machine-actionable fields (status, exact resolved versions, exported binary paths, resolved/missing libraries, and execution advice).

2. **Non-Interactive Fail-Safe Semantics**:
   When standard input is not an interactive TTY, or when `--non-interactive` / `--json` is specified, `pkg` must never block waiting on `stdin`. Any ambiguous decision or missing requirement must fail fast with structured exit codes and machine-parseable diagnostics so the agent can inspect and resolve programmatically.

3. **Native Model Context Protocol (MCP) Server (`pkg mcp`)**:
   `pkg` includes a first-class MCP server running over standard I/O, exposing typed tool definitions (`pkg.install`, `pkg.search`, `pkg.query_binary`, `pkg.create_profile`, `pkg.drop_profile`) directly to AI models without requiring shell string interpolation.

4. **Ephemeral Agent Workspaces (Scoped Profiles)**:
   The profile architecture supports lightweight, ephemeral task profiles (`pkg profile create <id>` / `pkg profile drop <id>`), allowing agents to provision temporary toolchains for isolated tasks and destroy them upon completion without leaving orphaned state on the host system.

5. **Capability and Command-Based Resolution**:
   Package resolution extends beyond nominal package names to support command-level and capability-level lookups (`pkg install --provides <command>`), allowing agents to request tools by their executable name rather than distribution-specific package naming quirks.

## Alternatives considered

- **Shell wrappers and external Python MCP bridges:** Rejected because external wrappers add brittle dependencies, do not eliminate interactive terminal hangs in the underlying engine, and lack direct visibility into the store and state database.
- **Human-only CLI with ad-hoc agent scraping:** Rejected because ANSI scraping and subprocess timeouts remain a primary failure mode for autonomous software agents in Linux.

## Why this decision

- Positions `pkg` as the premier package manager for both human engineers and AI coding assistants.
- Eliminates security risks associated with autonomous agents running `sudo` on host machines.
- Provides 100% deterministic, parseable outputs without terminal-hang edge cases.
- Enables agent sandboxing through disposable task profiles.

## Consequences

- All new commands and features must define both a human-facing terminal UX and a stable `--json` schema.
- Non-interactive safety checks must be enforced at the engine and CLI boundaries (see `INV-021`).
- The `--json` payload structure and MCP tool schemas become public API contracts subject to semantic versioning (see `INV-022`).

## Risks

- Maintaining dual interfaces (human terminal UX and machine JSON/MCP) increases API surface and testing requirements.
- Mitigated by deriving JSON output models directly from typed domain query and plan structures in `pkg-core`.

## Revisit conditions

- Revisit tool definitions as the Model Context Protocol specification evolves.
- Revisit non-interactive error code taxonomy as agent workflows expand.

## Evidence

- See `context/design/agent-first-architecture.md`, `context/project/invariants.md`, and `meta/decisions.md`.

## Confidence

HIGH
