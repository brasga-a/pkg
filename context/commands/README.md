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
| [`pkg update`](update.md) | Refresh repository metadata and snapshots. |
| [`pkg upgrade`](upgrade.md) | Plan and apply upgrades for installed packages. |
| [`pkg repo list`](repo-list.md) | List configured repositories and sync/trust status. |
| [`pkg repo add`](repo-add.md) | Add a repository configuration. |
| [`pkg doctor`](doctor.md) | Diagnose state/store/profile consistency and compatibility problems. |
| [`pkg gc`](gc.md) | Remove unreachable store objects and stale cache entries according to policy. |

## Global CLI principles

- Mutating commands must support `--dry-run` where a plan exists.
- Network operations must support an offline/fail-closed mode where meaningful.
- Human output is the default; machine-readable output can be added later without making internal serialization stable.
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
--json            # future stable-ish machine output, not internal plan schema
--dry-run         # only for commands that create a plan
```

Exact flag names remain provisional until the CLI ADR is revised with implementation feedback.
