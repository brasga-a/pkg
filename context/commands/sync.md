# `pkg repo sync`

Synchronize remote repository metadata and build immutable snapshots.

Analogous to syncing package databases (e.g. `pacman -Sy`), not upgrading installed packages.

`pkg sync` and `pkg repo update` remain compatibility aliases for this operation.

## Syntax

```bash
pkg repo sync
# aliases:
pkg sync
pkg repo update
```

## Flow

```text
repository config
 -> fetch metadata
 -> verify repository trust
 -> parse source metadata
 -> normalize candidates
 -> build immutable snapshot
 -> validate snapshot
 -> atomic active-snapshot switch
```

## Failure behavior

A failed refresh never destroys the previous valid snapshot.

For Debian repositories, each compressed index must match the size and SHA-256
recorded in the verified InRelease. Both xz and gzip require signed entries; a
valid signature on an unrelated InRelease alone does not authenticate an index.
Repository updates hold the same writer lock used by installation and recovery.

Examples of failure:
- network error;
- malformed metadata;
- expired/invalid signature;
- rollback/freeze policy violation;
- unsupported repository format.

## Cache behavior

Repository metadata cache is separate from artifact cache.

`pkg sync` does not download all package payloads.
