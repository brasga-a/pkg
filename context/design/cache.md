# Cache

Separate:

```text
cache/
├── metadata/
├── artifacts/
└── temp/
```

## Artifact cache identity

Prefer known cryptographic digest. If repository metadata does not provide a trustworthy digest:
- download to temporary file;
- compute digest;
- promote into digest-keyed cache.

URL is provenance, not identity.

## Partial downloads

Temporary names include transaction/request identity. Resume is allowed only when server semantics and expected identity make it safe.

## GC

Cache GC is independent from installed-store GC. Cached artifacts can be deleted without breaking installed packages.
