# Repository Research Notes

## Debian family

Repository indexes contain package metadata and paths/digests to artifacts. pkg can sync indexes, normalize them, and download selected `.deb` files only when needed.

## RPM family

DNF consumes repository metadata that describes RPM candidates and checksums. pkg should normalize repository candidates without opening the host RPM transaction database.

## ALPM

Arch sync databases are compressed tar metadata databases. They support search/dependency resolution separately from package artifact download.

## pkg-native repository

A future minimal design can be:

```text
index.json.zst
index.sig
packages/<digest>
```

or a sharded equivalent.

But a pkg-native protocol should not be finalized until adapters for real repositories establish which metadata fields are actually needed.
