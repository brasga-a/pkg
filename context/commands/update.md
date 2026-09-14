# `pkg update`

Refresh repository metadata.

This is analogous to refreshing package indexes, not upgrading installed packages.

## Syntax

```bash
pkg update
```

Potential targeting:

```bash
pkg update --repo debian
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

## Output

Suggested summary:

```text
debian-main    updated   83,214 packages
vendor         unchanged
arch-extra     failed    signature verification error
```

The overall exit status should communicate partial failure.

## Cache behavior

Repository metadata cache is separate from artifact cache.

`pkg update` does not download all package payloads.
