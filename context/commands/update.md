# `pkg update`

Resolve and apply newer versions of installed packages. This command is an
alias of [`pkg upgrade`](upgrade.md).

## Syntax

```bash
pkg update
pkg update <package>
pkg update --dry-run
pkg update --jobs 4
```

`pkg update` uses the same candidate resolution, download verification,
transaction, generation switch and rollback behavior as `pkg upgrade`.

`--jobs <N>` limits concurrent artifact acquisition to 1--16 tasks (default:
4). It parallelizes only downloads, verification and publication in the
digest-addressed cache. Installation, database writes and generation publication
remain serial and deterministic; `--jobs 1` is the serial reference mode.

To update repository metadata without changing installed packages, use
[`pkg repo sync`](sync.md).
