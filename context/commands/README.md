# CLI Commands

This directory defines the proposed user-facing command contract for `pkg`.

The CLI is intentionally narrower than the internal architecture. A documented command does not authorize unsupported package classes, native package-manager mutation, arbitrary lifecycle scripts, or host-wide changes outside accepted ADRs.

## Command index

| Command | Purpose |
|---|---|
| [`pkg install`](install.md) | Resolve or ingest a package and install it into the pkg store. |
| [`pkg remove`](remove.md) | Remove an installed package from the active profile and eligible store state. |
| [`pkg list`](list.md) | List locally installed packages and active versions. |
| [`pkg info`](info.md) | Show package metadata, provenance, compatibility and local state. |
| [`pkg search`](search.md) | Search synchronized repository catalogs. |
| [`pkg repo sync`](sync.md) | Refresh repository metadata and build snapshots. |
| [`pkg sync`](sync.md) | Compatibility alias for `pkg repo sync`. |
| [`pkg upgrade`](upgrade.md) | Plan and apply upgrades for installed packages. |
| [`pkg update`](update.md) | Compatibility alias for `pkg upgrade`. |
| [`pkg repo list`](repo-list.md) | List configured repositories and sync/trust status. |
| [`pkg repo add`](repo-add.md) | Add a repository configuration. |
| [`pkg repo update`](sync.md) | Compatibility alias for `pkg repo sync`. |
| [`pkg doctor`](doctor.md) | Diagnose state/store/profile consistency and compatibility problems. |
| [`pkg gc`](gc.md) | Remove unreachable store objects and stale cache entries according to policy. |
| [`pkg integrate`](integrate.md) | Explicitly expose supported desktop, icon and MIME resources in user space. |
| [`pkg deintegrate`](integrate.md) | Remove only host integration links owned by the selected package. |
| `pkg rollback` | Select a retained immutable profile generation and restore its recorded state. |
| `pkg run` | Execute a command through its recorded per-command runtime environment. |
| `pkg migrate` | Capture legacy activation as explicitly unverified evidence for later re-planning. |

| `pkg profile create|list|drop` | Manage isolated task profiles. |
| `pkg query-command <command>` | Find installed providers of a command. |
| `pkg mcp` | Serve the native JSON-RPC tool interface over STDIO. |

## Global CLI principles

- Mutating commands must support `--dry-run` where a plan exists.
- Network operations must support an offline/fail-closed mode where meaningful.
- Human output is the default; `--json` and MCP expose typed machine-readable contracts.
- Errors must be categorized, not collapsed into a single generic failure.
- Commands never execute package metadata through a shell.
- A command succeeds only after its durable/visible postconditions are met.
- Unsupported compatibility is reported explicitly rather than guessed into success.

## Suggested global flags

```text
--help
--version
--verbose
--quiet
--offline
--profile <name>
--json            # one deterministic machine-readable result document
--dry-run         # only for commands that create a plan
```

Exact flag names remain provisional until the CLI ADR is revised with implementation feedback.
