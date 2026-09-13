# `pkg install`

Install a package from a repository name, local artifact, or explicit URL.

## Syntax

```bash
pkg install <package>
pkg install <path>
pkg install <url>
```

Examples:

```bash
pkg install ripgrep
pkg install ./vendor.deb
pkg install https://example.org/vendor.deb
pkg install ripgrep --dry-run
```

## Accepted input classes

### Repository package name

```bash
pkg install ripgrep
```

Resolution path:

```text
name
 -> active repository snapshots
 -> candidate selection
 -> dependency resolution
 -> compatibility planning
 -> artifact fetch
 -> verification
 -> transaction
```

### Local artifact

```bash
pkg install ./app.deb
```

Resolution path:

```text
local file
 -> format probe
 -> metadata parse
 -> local digest
 -> compatibility planning
 -> transaction
```

### Explicit URL

```bash
pkg install https://vendor.example/app.deb
```

The URL is provenance, not package identity. The artifact receives a local cryptographic digest before promotion.

## High-level flow

```text
Input
  |
Resolve source
  |
Read/normalize metadata
  |
Check architecture
  |
Resolve dependencies/capabilities
  |
Inspect scripts/integration requirements
  |
Inspect ELF/runtime requirements
  |
Create InstallPlan
  |
Verify artifact/trust
  |
Stage payload
  |
Promote store object
  |
Activate profile entries
  |
Commit state
```

## Install plan

Before mutation, `pkg` should know:

- package name/version;
- source repository or local provenance;
- source artifact format;
- artifact digest;
- target store object;
- dependencies and their selected providers;
- host capabilities used;
- ignored/blocked maintainer scripts;
- binary activation entries;
- conflicts;
- unsupported integrations.

`--dry-run` stops after producing this plan.

## Package script policy

Foreign maintainer scripts are never run by this command in the initial architecture.

If the package requires one for correctness:

```text
NeedsHostIntegration
```

or:

```text
Unsupported
```

is returned.

## Version behavior

Repository installation selects a version according to the active resolver policy.

Potential future syntax:

```bash
pkg install foo@1.2.3
```

The internal model must allow side-by-side store versions even if only one version is active in a profile.

## Activation conflicts

If two active packages provide the same command:

```text
error: command `foo` is already provided by package A
```

The command fails unless an explicit future conflict-selection mechanism is used.

## Postconditions

Success means:

- artifact integrity was accepted;
- final store object exists;
- package state is committed;
- requested activation is visible in the selected profile;
- transaction is `Completed`.

Successful extraction alone is not success.

## Failure classes

- package not found;
- unsupported artifact;
- malformed artifact;
- architecture mismatch;
- unsatisfied dependency;
- incompatible host ABI;
- blocked lifecycle script;
- trust/signature failure;
- digest mismatch;
- activation conflict;
- filesystem/store error;
- transaction recovery required.

## Security invariants

- no direct package payload writes to `/usr`;
- no arbitrary scripts;
- no archive path traversal;
- no shell evaluation;
- no native package database mutation;
- rootless by default.
